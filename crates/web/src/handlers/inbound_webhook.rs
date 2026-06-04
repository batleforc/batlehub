use std::sync::Arc;

use actix_web::{get, post, web, HttpRequest, HttpResponse, Responder};
use batlehub_config::schema::NotificationsConfig;
use batlehub_core::entities::InboundWebhookEvent;
use bytes::BytesMut;
use chrono::Utc;
use futures::StreamExt;
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    error::AppError, extractors::AuthIdentity, handlers::back_office::require_admin,
    services::verify_inbound_hmac,
};
use batlehub_core::ports::NotificationPort;

// ── Inbound webhook receiver ──────────────────────────────────────────────────

/// Receive an event from an external system.
///
/// If a `secret` is configured for this webhook, the request must include a
/// `X-Hub-Signature-256: sha256=<hmac-hex>` header computed over the raw body.
#[utoipa::path(
    post,
    path = "/api/v1/webhooks/inbound/{name}",
    tag = "notifications",
    params(("name" = String, Path, description = "Inbound webhook name")),
    responses(
        (status = 200, description = "Event received"),
        (status = 400, description = "Unknown webhook name"),
        (status = 401, description = "HMAC signature mismatch"),
    ),
)]
#[post("/api/v1/webhooks/inbound/{name}")]
pub async fn receive_inbound_webhook(
    req: HttpRequest,
    path: web::Path<String>,
    mut payload: web::Payload,
    notification_store: web::Data<Arc<dyn NotificationPort>>,
    notifications_config: web::Data<Option<NotificationsConfig>>,
) -> Result<impl Responder, AppError> {
    let name = path.into_inner();

    let mut raw = BytesMut::new();
    while let Some(chunk) = payload.next().await {
        let chunk = chunk.map_err(|e| AppError::bad_request(e.to_string()))?;
        raw.extend_from_slice(&chunk);
    }
    let body = raw.freeze();

    let inbound_cfg = notifications_config
        .as_ref()
        .as_ref()
        .and_then(|nc| nc.inbound.iter().find(|i| i.name == name));

    if inbound_cfg.is_none() {
        // Be vague about whether it exists to avoid enumeration.
        return Err(AppError::bad_request(format!(
            "unknown inbound webhook: {name}"
        )));
    }
    let inbound_cfg = inbound_cfg.unwrap();

    // Empty secrets provide no security (attacker can compute HMAC with empty key),
    // so treat them the same as no secret configured.
    let signature_valid =
        match inbound_cfg.secret.as_deref().filter(|s| !s.is_empty()) {
            Some(secret) => {
                let header_val = req
                    .headers()
                    .get("X-Hub-Signature-256")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                if !verify_inbound_hmac(secret, &body, header_val) {
                    return Err(AppError::forbidden("HMAC signature mismatch"));
                }
                Some(true)
            }
            None => None,
        };

    let payload: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| AppError::bad_request(format!("request body is not valid JSON: {e}")))?;

    let source_ip = req
        .connection_info()
        .realip_remote_addr()
        .map(str::to_owned);

    let event = InboundWebhookEvent {
        id: Uuid::new_v4(),
        webhook_name: name,
        payload,
        source_ip,
        received_at: Utc::now(),
        signature_valid,
    };

    notification_store
        .record_inbound_event(event)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;

    Ok(HttpResponse::Ok().finish())
}

// ── Admin: list inbound events ────────────────────────────────────────────────

#[derive(Debug, Serialize, ToSchema)]
pub struct InboundEventsResponse {
    pub events: Vec<batlehub_core::entities::InboundWebhookEvent>,
}

/// List recent inbound webhook events (admin only).
#[utoipa::path(
    get,
    path = "/api/v1/admin/notifications/inbound",
    tag = "back-office",
    responses(
        (status = 200, description = "Inbound events", body = InboundEventsResponse),
        (status = 403, description = "Admin role required"),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/admin/notifications/inbound")]
pub async fn list_inbound_events(
    identity: AuthIdentity,
    notification_store: web::Data<Arc<dyn NotificationPort>>,
) -> Result<impl Responder, AppError> {
    require_admin(&identity)?;
    let events = notification_store
        .list_inbound_events(100)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(web::Json(InboundEventsResponse { events }))
}
