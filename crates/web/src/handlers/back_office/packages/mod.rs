use std::sync::Arc;

use actix_web::{get, post, web, Responder};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use batlehub_core::{
    entities::{PackageFilter, PackageId},
    services::{AdminService, BulkBlockItem, ProxyService},
};

use super::require_admin;
use crate::{error::AppError, extractors::AuthIdentity};

pub mod bulk;
pub mod detail;
pub mod list;

pub use bulk::{
    bulk_block_packages, bulk_delete_packages, bulk_unblock_packages, BulkBlockRequest,
    BulkBlockRequestItem, BulkDeleteRequest, BulkDeleteRequestItem, BulkUnblockRequest,
    BulkUnblockRequestItem,
};
pub use detail::{
    package_detail, PackageDetailQuery, PackageDetailResponse, PackageEventDto,
    PackageStatusDetail, PackageVersionDetail, VulnerabilityDto,
};
pub use list::{
    block_package, delete_package, invalidate_package, list_packages, unblock_package,
    AdminPackageQuery, BlockRequest, DeletePackageRequest, InvalidateRequest, UnblockRequest,
};

// ── Shared DTOs ───────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct ActionResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Serialize, ToSchema)]
pub struct BulkFailureDto {
    pub registry: String,
    pub name: String,
    pub version: String,
    pub artifact: Option<String>,
    pub error: String,
}

#[derive(Serialize, ToSchema)]
pub struct BulkActionResponse {
    pub succeeded_count: usize,
    pub failed_count: usize,
    pub failures: Vec<BulkFailureDto>,
}

pub fn map_bulk_failures(failed: Vec<(PackageId, String)>) -> Vec<BulkFailureDto> {
    failed
        .into_iter()
        .map(|(pkg, error)| BulkFailureDto {
            registry: pkg.registry,
            name: pkg.name,
            version: pkg.version,
            artifact: pkg.artifact,
            error,
        })
        .collect()
}

fn default_per_page() -> u64 {
    50
}
