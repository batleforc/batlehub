use std::sync::Arc;

use actix_web::{get, web, Responder};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use batlehub_core::{
    entities::{ExploreFilter, ExploreSortBy, PackageFilter, PackageSource, PackageStatus},
    services::{AdminService, LocalRegistryService, ProxyService},
};

use crate::{error::AppError, extractors::AuthIdentity};

pub mod detail;
pub mod list;
pub mod stats;

pub use detail::{
    explore_package_detail, ExplorePackageDetailResponse, ExploreVersionDto, FirewallDto, GateDto,
    PackageDetailPath,
};
pub use list::{explore_packages, ExploreEntryDto, ExplorePackageListResponse, ExploreQuery};
pub use stats::{
    explore_registry_stats, explore_upstream_search, ExploreRegistryStatsResponse, RegistryStatDto,
    UpstreamPackageDto, UpstreamSearchQuery, UpstreamSearchResponse,
};

pub fn format_dt(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339()
}

pub fn default_per_page() -> u64 {
    20
}
