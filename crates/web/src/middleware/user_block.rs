use std::future::{ready, Ready};
use std::rc::Rc;
use std::sync::Arc;

use actix_web::{
    body::{BoxBody, MessageBody},
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage, HttpResponse,
};
use futures::future::LocalBoxFuture;

use batlehub_core::{entities::Identity, ports::UserBlockRepository};

// ── Factory ───────────────────────────────────────────────────────────────────

/// Middleware that rejects requests from users whose `user_id` appears in the
/// `user_blocks` table.
///
/// Must be placed so that it runs **after** `AuthMiddlewareFactory` in the
/// request pipeline (i.e., added with `.wrap()` _before_ `AuthMiddlewareFactory`
/// in the builder chain, since actix-web reverses wrap order).
///
/// Anonymous identities (no `user_id`) are always allowed through.
pub struct UserBlockMiddlewareFactory {
    repo: Arc<dyn UserBlockRepository>,
}

impl UserBlockMiddlewareFactory {
    pub fn new(repo: Arc<dyn UserBlockRepository>) -> Self {
        Self { repo }
    }
}

impl<S, B> Transform<S, ServiceRequest> for UserBlockMiddlewareFactory
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type InitError = ();
    type Transform = UserBlockMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(UserBlockMiddleware {
            service: Rc::new(service),
            repo: self.repo.clone(),
        }))
    }
}

pub struct UserBlockMiddleware<S> {
    service: Rc<S>,
    repo: Arc<dyn UserBlockRepository>,
}

impl<S, B> Service<ServiceRequest> for UserBlockMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();
        let repo = self.repo.clone();

        Box::pin(async move {
            let user_id = req
                .extensions()
                .get::<Identity>()
                .and_then(|id| id.user_id.clone());

            if let Some(uid) = user_id {
                match repo.is_blocked(&uid).await {
                    Ok(true) => {
                        tracing::debug!(user_id = %uid, "blocked user rejected");
                        let response = HttpResponse::Unauthorized()
                            .json(serde_json::json!({ "error": "user account is blocked" }));
                        return Ok(req.into_response(response).map_into_boxed_body());
                    }
                    Ok(false) => {}
                    Err(e) => {
                        tracing::warn!(user_id = %uid, error = %e, "user block check failed, allowing through");
                    }
                }
            }

            service
                .call(req)
                .await
                .map(|r| r.map_into_boxed_body())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{
        dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
        test::{self, TestRequest},
        web, App, Error, HttpResponse,
    };
    use batlehub_adapters::db::InMemoryUserBlockRepository;
    use batlehub_core::entities::{Identity, Role};
    use batlehub_core::ports::UserBlockRepository;

    fn make_identity(user_id: Option<&str>) -> Identity {
        Identity {
            user_id: user_id.map(str::to_owned),
            role: Role::User,
            auth_provider: None,
            groups: vec![],
        }
    }

    // ── Minimal test-only middleware that injects an Identity into request extensions ──
    //
    // Using a simple passthrough (Response = ServiceResponse<B>) avoids chaining two
    // EitherBody-transforming middlewares, which causes type mismatches with init_service.

    struct InjectIdentity(Option<Identity>);

    impl<S, B> Transform<S, ServiceRequest> for InjectIdentity
    where
        S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
        B: 'static,
    {
        type Response = ServiceResponse<B>;
        type Error = Error;
        type InitError = ();
        type Transform = InjectIdentityMiddleware<S>;
        type Future = std::future::Ready<Result<Self::Transform, ()>>;

        fn new_transform(&self, service: S) -> Self::Future {
            std::future::ready(Ok(InjectIdentityMiddleware {
                service: Rc::new(service),
                identity: self.0.clone(),
            }))
        }
    }

    struct InjectIdentityMiddleware<S> {
        service: Rc<S>,
        identity: Option<Identity>,
    }

    impl<S, B> Service<ServiceRequest> for InjectIdentityMiddleware<S>
    where
        S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
        B: 'static,
    {
        type Response = ServiceResponse<B>;
        type Error = Error;
        type Future = futures::future::LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

        forward_ready!(service);

        fn call(&self, req: ServiceRequest) -> Self::Future {
            if let Some(ref id) = self.identity {
                req.extensions_mut().insert(id.clone());
            }
            let fut = self.service.call(req);
            Box::pin(async move { fut.await })
        }
    }

    // ── Tests ─────────────────────────────────────────────────────────────────────

    #[actix_web::test]
    async fn anonymous_is_allowed() {
        let repo: Arc<dyn UserBlockRepository> = Arc::new(InMemoryUserBlockRepository::new());
        let app = test::init_service(
            App::new()
                .wrap(UserBlockMiddlewareFactory::new(Arc::clone(&repo)))
                .wrap(InjectIdentity(None))
                .route("/", web::get().to(|| async { HttpResponse::Ok().finish() })),
        )
        .await;
        let req = TestRequest::get().uri("/").to_request();
        assert_eq!(test::call_service(&app, req).await.status(), 200);
    }

    #[actix_web::test]
    async fn unblocked_user_is_allowed() {
        let repo: Arc<dyn UserBlockRepository> = Arc::new(InMemoryUserBlockRepository::new());
        let app = test::init_service(
            App::new()
                .wrap(UserBlockMiddlewareFactory::new(Arc::clone(&repo)))
                .wrap(InjectIdentity(Some(make_identity(Some("alice")))))
                .route("/", web::get().to(|| async { HttpResponse::Ok().finish() })),
        )
        .await;
        let req = TestRequest::get().uri("/").to_request();
        assert_eq!(test::call_service(&app, req).await.status(), 200);
    }

    #[actix_web::test]
    async fn blocked_user_is_rejected() {
        let repo: Arc<dyn UserBlockRepository> = Arc::new(InMemoryUserBlockRepository::new());
        repo.block("blocked-user", "admin", Some("test")).await.unwrap();
        let app = test::init_service(
            App::new()
                .wrap(UserBlockMiddlewareFactory::new(Arc::clone(&repo)))
                .wrap(InjectIdentity(Some(make_identity(Some("blocked-user")))))
                .route("/", web::get().to(|| async { HttpResponse::Ok().finish() })),
        )
        .await;
        let req = TestRequest::get().uri("/").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), 401);
    }

    // When `is_blocked` returns an error the middleware must allow the request
    // through rather than crashing or returning 500 (fail-open to avoid DoS on
    // DB hiccups).
    #[actix_web::test]
    async fn block_check_error_allows_request_through() {
        struct FailingRepo;

        #[async_trait::async_trait]
        impl UserBlockRepository for FailingRepo {
            async fn list(
                &self,
            ) -> Result<Vec<batlehub_core::ports::UserBlock>, batlehub_core::error::CoreError>
            {
                Ok(vec![])
            }
            async fn block(
                &self,
                _: &str,
                _: &str,
                _: Option<&str>,
            ) -> Result<(), batlehub_core::error::CoreError> {
                Ok(())
            }
            async fn unblock(
                &self,
                _: &str,
            ) -> Result<(), batlehub_core::error::CoreError> {
                Ok(())
            }
            async fn is_blocked(
                &self,
                _: &str,
            ) -> Result<bool, batlehub_core::error::CoreError> {
                Err(batlehub_core::error::CoreError::Database(
                    "simulated DB error".into(),
                ))
            }
        }

        let repo: Arc<dyn UserBlockRepository> = Arc::new(FailingRepo);
        let app = test::init_service(
            App::new()
                .wrap(UserBlockMiddlewareFactory::new(Arc::clone(&repo)))
                .wrap(InjectIdentity(Some(make_identity(Some("any-user")))))
                .route("/", web::get().to(|| async { HttpResponse::Ok().finish() })),
        )
        .await;
        let req = TestRequest::get().uri("/").to_request();
        // Should still return 200 — fail-open on repository errors.
        assert_eq!(test::call_service(&app, req).await.status(), 200);
    }
}
