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

pub use detail::*;
pub use list::*;
pub use stats::*;

pub fn format_dt(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339()
}

pub fn default_per_page() -> u64 {
    20
}
