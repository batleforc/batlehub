use std::future::{ready, Ready};
use std::rc::Rc;
use std::sync::Arc;

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage,
};
use futures::future::LocalBoxFuture;

use batlehub_core::{entities::Identity, ports::AuthProvider};

use crate::extractors::raw_auth_from_request;

/// Actix-web middleware that attempts each configured `AuthProvider` in order.
///
/// On success the resolved `Identity` is stored in request extensions so that
/// handlers can extract it via `AuthIdentity`. Falls back to `Identity::anonymous()`.
pub struct AuthMiddlewareFactory {
    providers: Arc<Vec<Arc<dyn AuthProvider>>>,
}

impl AuthMiddlewareFactory {
    pub fn new(providers: Vec<Arc<dyn AuthProvider>>) -> Self {
        Self {
            providers: Arc::new(providers),
        }
    }
}

impl<S, B> Transform<S, ServiceRequest> for AuthMiddlewareFactory
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = AuthMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(AuthMiddleware {
            service: Rc::new(service),
            providers: self.providers.clone(),
        }))
    }
}

pub struct AuthMiddleware<S> {
    service: Rc<S>,
    providers: Arc<Vec<Arc<dyn AuthProvider>>>,
}

impl<S, B> Service<ServiceRequest> for AuthMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();
        let providers = self.providers.clone();

        Box::pin(async move {
            let raw = raw_auth_from_request(req.request());
            let mut identity = Identity::anonymous();

            'providers: for provider in providers.iter() {
                match provider.authenticate(&raw).await {
                    Ok(Some(id)) => {
                        tracing::debug!(
                            provider = provider.name(),
                            user_id = ?id.user_id,
                            role = %id.role,
                            "authenticated"
                        );
                        identity = id;
                        break 'providers;
                    }
                    Ok(None) => {} // provider did not recognise the credentials
                    Err(e) => {
                        // Only genuine provider-level failures land here (e.g. malformed
                        // JWT, unknown signing key) — a wrong static token or an expired
                        // JWT returns `Ok(None)` per the `AuthProvider` contract, so this
                        // counter deliberately does not cover "all failed logins".
                        metrics::counter!("batlehub_auth_failures_total", "provider" => provider.name().to_string()).increment(1);
                        tracing::warn!(provider = provider.name(), error = %e, "auth provider error");
                    }
                }
            }

            req.extensions_mut().insert(identity);
            service.call(req).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{
        test::{self, TestRequest},
        web, App, HttpRequest, HttpResponse,
    };
    use async_trait::async_trait;
    use batlehub_core::{
        entities::{Identity, Role},
        error::CoreError,
        ports::{AuthProvider, RawAuthRequest},
    };

    struct AlwaysAuth(String);

    #[async_trait]
    impl AuthProvider for AlwaysAuth {
        fn name(&self) -> &str {
            "always"
        }
        async fn authenticate(&self, _: &RawAuthRequest) -> Result<Option<Identity>, CoreError> {
            Ok(Some(Identity {
                user_id: Some(self.0.clone()),
                role: Role::User,
                auth_provider: Some("always".into()),
                groups: vec![],
            }))
        }
    }

    struct NeverAuth;

    #[async_trait]
    impl AuthProvider for NeverAuth {
        fn name(&self) -> &str {
            "never"
        }
        async fn authenticate(&self, _: &RawAuthRequest) -> Result<Option<Identity>, CoreError> {
            Ok(None)
        }
    }

    async fn who_am_i(req: HttpRequest) -> HttpResponse {
        let user = req
            .extensions()
            .get::<Identity>()
            .and_then(|i| i.user_id.clone())
            .unwrap_or_else(|| "anonymous".into());
        HttpResponse::Ok().body(user)
    }

    #[actix_web::test]
    async fn no_providers_yields_anonymous() {
        let app = test::init_service(
            App::new()
                .wrap(AuthMiddlewareFactory::new(vec![]))
                .route("/", web::get().to(who_am_i)),
        )
        .await;
        let req = TestRequest::get().uri("/").to_request();
        let resp = test::call_service(&app, req).await;
        let body = test::read_body(resp).await;
        assert_eq!(body, "anonymous");
    }

    #[actix_web::test]
    async fn first_matching_provider_wins() {
        let providers: Vec<Arc<dyn AuthProvider>> = vec![
            Arc::new(AlwaysAuth("alice".into())),
            Arc::new(AlwaysAuth("bob".into())),
        ];
        let app = test::init_service(
            App::new()
                .wrap(AuthMiddlewareFactory::new(providers))
                .route("/", web::get().to(who_am_i)),
        )
        .await;
        let req = TestRequest::get().uri("/").to_request();
        let body = test::read_body(test::call_service(&app, req).await).await;
        assert_eq!(body, "alice");
    }

    #[actix_web::test]
    async fn falls_back_to_second_provider() {
        let providers: Vec<Arc<dyn AuthProvider>> =
            vec![Arc::new(NeverAuth), Arc::new(AlwaysAuth("carol".into()))];
        let app = test::init_service(
            App::new()
                .wrap(AuthMiddlewareFactory::new(providers))
                .route("/", web::get().to(who_am_i)),
        )
        .await;
        let req = TestRequest::get().uri("/").to_request();
        let body = test::read_body(test::call_service(&app, req).await).await;
        assert_eq!(body, "carol");
    }

    #[actix_web::test]
    async fn all_providers_fail_yields_anonymous() {
        let providers: Vec<Arc<dyn AuthProvider>> = vec![Arc::new(NeverAuth)];
        let app = test::init_service(
            App::new()
                .wrap(AuthMiddlewareFactory::new(providers))
                .route("/", web::get().to(who_am_i)),
        )
        .await;
        let req = TestRequest::get().uri("/").to_request();
        let body = test::read_body(test::call_service(&app, req).await).await;
        assert_eq!(body, "anonymous");
    }
}
