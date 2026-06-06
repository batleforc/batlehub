use std::sync::Arc;

use actix_web::{delete, get, post, put, web, HttpResponse, Responder};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use batlehub_core::entities::{NotificationEventType, NotificationSubscription};

use super::require_admin;
use crate::{error::AppError, extractors::AuthIdentity, services::NotificationService};

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSubscriptionRequest {
    /// When `null`, applies to all registries.
    pub registry: Option<String>,
    /// When `null`, applies to all packages in the selected registries.
    pub package_name: Option<String>,
    pub event_types: Vec<NotificationEventType>,
    pub channel_name: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSubscriptionRequest {
    pub registry: Option<String>,
    pub package_name: Option<String>,
    pub event_types: Vec<NotificationEventType>,
    pub channel_name: String,
    pub enabled: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChannelListResponse {
    pub channels: Vec<ChannelInfo>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ChannelInfo {
    pub name: String,
}

// ── Guard ─────────────────────────────────────────────────────────────────────

fn require_notifications(
    svc: &Option<Arc<NotificationService>>,
) -> Result<&Arc<NotificationService>, AppError> {
    svc.as_ref()
        .ok_or_else(|| AppError::service_unavailable("notifications not configured"))
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// List configured notification channels (names only — no secrets).
#[utoipa::path(
    get,
    path = "/api/v1/admin/notifications/channels",
    tag = "back-office",
    responses(
        (status = 200, description = "Channel list", body = ChannelListResponse),
        (status = 403, description = "Admin role required"),
        (status = 503, description = "Notifications not configured"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/notifications/channels")]
pub async fn list_notification_channels(
    identity: AuthIdentity,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let svc = require_notifications(&notification_svc)?;
    let channels = svc
        .channel_names()
        .into_iter()
        .map(|name| ChannelInfo { name })
        .collect();
    Ok(web::Json(ChannelListResponse { channels }))
}

/// List all notification subscriptions.
#[utoipa::path(
    get,
    path = "/api/v1/admin/notifications/subscriptions",
    tag = "back-office",
    responses(
        (status = 200, description = "Subscription list", body = Vec<NotificationSubscription>),
        (status = 403, description = "Admin role required"),
        (status = 503, description = "Notifications not configured"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/notifications/subscriptions")]
pub async fn list_subscriptions(
    identity: AuthIdentity,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let svc = require_notifications(&notification_svc)?;
    let subs = svc
        .list_subscriptions()
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(web::Json(subs))
}

/// Create a notification subscription.
#[utoipa::path(
    post,
    path = "/api/v1/admin/notifications/subscriptions",
    tag = "back-office",
    request_body = CreateSubscriptionRequest,
    responses(
        (status = 201, description = "Subscription created", body = NotificationSubscription),
        (status = 400, description = "Invalid request"),
        (status = 403, description = "Admin role required"),
        (status = 503, description = "Notifications not configured"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/notifications/subscriptions")]
pub async fn create_subscription(
    identity: AuthIdentity,
    body: web::Json<CreateSubscriptionRequest>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let svc = require_notifications(&notification_svc)?;
    if body.event_types.is_empty() {
        return Err(AppError::bad_request("event_types must not be empty"));
    }
    if !svc.channel_names().contains(&body.channel_name) {
        return Err(AppError::bad_request(format!(
            "unknown channel_name '{}': not present in server configuration",
            body.channel_name
        )));
    }
    let actor = identity
        .0
        .user_id
        .clone()
        .unwrap_or_else(|| "admin".to_owned());
    let sub = NotificationSubscription {
        id: Uuid::new_v4(),
        registry: body.registry.clone(),
        package_name: body.package_name.clone(),
        event_types: body.event_types.clone(),
        channel_name: body.channel_name.clone(),
        created_by: actor,
        created_at: Utc::now(),
        enabled: true,
    };
    svc.add_subscription(sub.clone())
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(HttpResponse::Created().json(sub))
}

/// Get a single notification subscription by ID.
#[utoipa::path(
    get,
    path = "/api/v1/admin/notifications/subscriptions/{id}",
    tag = "back-office",
    params(("id" = Uuid, Path, description = "Subscription ID")),
    responses(
        (status = 200, description = "Subscription", body = NotificationSubscription),
        (status = 403, description = "Admin role required"),
        (status = 404, description = "Not found"),
        (status = 503, description = "Notifications not configured"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/notifications/subscriptions/{id}")]
pub async fn get_subscription(
    identity: AuthIdentity,
    path: web::Path<Uuid>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let svc = require_notifications(&notification_svc)?;
    let id = path.into_inner();
    let sub = svc
        .get_subscription(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found(format!("subscription {id}")))?;
    Ok(web::Json(sub))
}

/// Update a notification subscription.
#[utoipa::path(
    put,
    path = "/api/v1/admin/notifications/subscriptions/{id}",
    tag = "back-office",
    params(("id" = Uuid, Path, description = "Subscription ID")),
    request_body = UpdateSubscriptionRequest,
    responses(
        (status = 200, description = "Updated", body = NotificationSubscription),
        (status = 400, description = "Invalid request"),
        (status = 403, description = "Admin role required"),
        (status = 404, description = "Not found"),
        (status = 503, description = "Notifications not configured"),
    ),
    security(("bearer_token" = [])),
)]
#[put("/api/v1/admin/notifications/subscriptions/{id}")]
pub async fn update_subscription(
    identity: AuthIdentity,
    path: web::Path<Uuid>,
    body: web::Json<UpdateSubscriptionRequest>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let svc = require_notifications(&notification_svc)?;
    let id = path.into_inner();
    if body.event_types.is_empty() {
        return Err(AppError::bad_request("event_types must not be empty"));
    }
    if !svc.channel_names().contains(&body.channel_name) {
        return Err(AppError::bad_request(format!(
            "unknown channel_name '{}': not present in server configuration",
            body.channel_name
        )));
    }

    // Fetch existing to preserve immutable fields.
    let existing = svc
        .get_subscription(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found(format!("subscription {id}")))?;

    let updated = NotificationSubscription {
        registry: body.registry.clone(),
        package_name: body.package_name.clone(),
        event_types: body.event_types.clone(),
        channel_name: body.channel_name.clone(),
        enabled: body.enabled,
        ..existing
    };
    svc.update_subscription(updated.clone())
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(web::Json(updated))
}

/// Delete a notification subscription.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/notifications/subscriptions/{id}",
    tag = "back-office",
    params(("id" = Uuid, Path, description = "Subscription ID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 403, description = "Admin role required"),
        (status = 404, description = "Not found"),
        (status = 503, description = "Notifications not configured"),
    ),
    security(("bearer_token" = [])),
)]
#[delete("/api/v1/admin/notifications/subscriptions/{id}")]
pub async fn delete_subscription(
    identity: AuthIdentity,
    path: web::Path<Uuid>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let svc = require_notifications(&notification_svc)?;
    let id = path.into_inner();
    svc.get_subscription(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found(format!("subscription {id}")))?;
    svc.remove_subscription(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(HttpResponse::NoContent().finish())
}

/// Send a test notification for a subscription.
#[utoipa::path(
    post,
    path = "/api/v1/admin/notifications/subscriptions/{id}/test",
    tag = "back-office",
    params(("id" = Uuid, Path, description = "Subscription ID")),
    responses(
        (status = 200, description = "Test sent"),
        (status = 400, description = "Dispatch failed"),
        (status = 403, description = "Admin role required"),
        (status = 404, description = "Not found"),
        (status = 503, description = "Notifications not configured"),
    ),
    security(("bearer_token" = [])),
)]
#[post("/api/v1/admin/notifications/subscriptions/{id}/test")]
pub async fn test_subscription(
    identity: AuthIdentity,
    path: web::Path<Uuid>,
    notification_svc: web::Data<Option<Arc<NotificationService>>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let svc = require_notifications(&notification_svc)?;
    let id = path.into_inner();
    let sub = svc
        .get_subscription(id)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .ok_or_else(|| AppError::not_found(format!("subscription {id}")))?;
    svc.test_subscription(&sub).await.map_err(|e| {
        tracing::warn!(
            subscription_id = %id,
            channel = %sub.channel_name,
            "test dispatch failed: {e}"
        );
        AppError::bad_request("test dispatch failed; check server logs for details")
    })?;
    Ok(HttpResponse::Ok().finish())
}
