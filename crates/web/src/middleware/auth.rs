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
                        tracing::warn!(provider = provider.name(), error = %e, "auth provider error");
                    }
                }
            }

            req.extensions_mut().insert(identity);
            service.call(req).await
        })
    }
}
