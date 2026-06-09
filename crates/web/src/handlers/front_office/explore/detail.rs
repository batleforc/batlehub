use super::{
    format_dt, get, web, AdminService, AppError, Arc, AuthIdentity, Deserialize, IntoParams,
    LocalRegistryService, PackageFilter, PackageStatus, Responder, Serialize, ToSchema,
};

// ── Package detail ─────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
pub struct PackageDetailPath {
    pub registry: String,
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct ExplorePackageDetailResponse {
    pub registry: String,
    pub name: String,
    pub gate: GateDto,
    pub versions: Vec<ExploreVersionDto>,
    /// `true` when the upstream database was unreachable and this package has no cached data.
    pub upstream_unavailable: bool,
}

#[derive(Serialize, ToSchema)]
pub struct GateDto {
    /// Whether the caller's role can access this registry through the proxy.
    pub registry_accessible: bool,
    /// Whether the caller is a beta-channel member for this registry.
    pub beta_member: bool,
}

#[derive(Serialize, ToSchema)]
pub struct ExploreVersionDto {
    pub version: String,
    /// `"proxied"` | `"local"`
    pub source: String,
    pub firewall: FirewallDto,
    pub download_count: u64,
    pub last_accessed: Option<String>,
    pub published_at: Option<String>,
    pub is_prerelease: bool,
}

#[derive(Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum FirewallDto {
    Clear,
    Blocked {
        reason: String,
        blocked_by: String,
        blocked_at: String,
    },
    Yanked,
}

/// Package detail view: all known versions with gate and firewall status.
#[utoipa::path(
    get,
    path = "/api/v1/explore/packages/{registry}/{name}",
    tag = "explore",
    params(PackageDetailPath),
    responses(
        (status = 200, description = "Package detail", body = ExplorePackageDetailResponse),
    ),
    security(("bearer_token" = [])),
)]
#[get("/api/v1/explore/packages/{registry}/{name}")]
pub async fn explore_package_detail(
    path: web::Path<PackageDetailPath>,
    identity: AuthIdentity,
    admin_svc: web::Data<Arc<AdminService>>,
    local_svc: web::Data<Arc<LocalRegistryService>>,
    access: web::Data<crate::AccessConfigLock>,
) -> Result<impl Responder, AppError> {
    let registry = &path.registry;
    let name = &path.name;

    // Gate: registry-level proxy access
    let registry_accessible = access
        .read()
        .await
        .accessible_registries_for(&identity)
        .contains(registry);

    // Gate: beta channel membership
    let beta_member = {
        let beta_port = local_svc
            .hot
            .read()
            .await
            .beta_channel
            .get(registry)
            .cloned();
        if let Some(bp) = beta_port {
            bp.is_member(registry, &identity).await.unwrap_or(false)
        } else {
            false
        }
    };

    // Proxied versions from package_statuses
    let proxied_filter = PackageFilter {
        registry: Some(registry.clone()),
        registries: vec![],
        name_exact: Some(name.clone()),
        name_contains: None,
        blocked_only: false,
        limit: 500,
        offset: 0,
    };
    let (proxied_summaries, upstream_unavailable) =
        match admin_svc.list_packages(proxied_filter).await {
            Ok(summaries) => (summaries, false),
            Err(_) => (vec![], true),
        };

    // Local versions from local_packages
    let local_versions = local_svc
        .backend
        .get_versions(registry, name)
        .await
        .unwrap_or_default();

    // Build version entries
    let mut versions: Vec<ExploreVersionDto> = Vec::new();

    // Track which versions came from local to avoid duplicating proxied entries
    let local_version_set: std::collections::HashSet<&str> =
        local_versions.iter().map(|v| v.version.as_str()).collect();

    for summary in proxied_summaries {
        // Skip versions also present in local (they'll appear as "local")
        if local_version_set.contains(summary.package_id.version.as_str()) {
            continue;
        }
        let firewall = match summary.status {
            PackageStatus::Available => FirewallDto::Clear,
            PackageStatus::Blocked {
                reason,
                blocked_by,
                blocked_at,
            } => FirewallDto::Blocked {
                reason,
                blocked_by,
                blocked_at: blocked_at.to_rfc3339(),
            },
        };
        let is_prerelease = summary.package_id.version.contains('-');
        versions.push(ExploreVersionDto {
            version: summary.package_id.version,
            source: "proxied".to_string(),
            firewall,
            download_count: summary.access_count,
            last_accessed: summary.last_accessed.map(format_dt),
            published_at: None,
            is_prerelease,
        });
    }

    for pkg in local_versions {
        let firewall = if pkg.yanked {
            FirewallDto::Yanked
        } else {
            FirewallDto::Clear
        };
        let is_prerelease = pkg.version.contains('-');
        versions.push(ExploreVersionDto {
            version: pkg.version,
            source: "local".to_string(),
            firewall,
            download_count: 0,
            last_accessed: None,
            published_at: Some(pkg.published_at.to_rfc3339()),
            is_prerelease,
        });
    }

    // Sort: stable versions first, then pre-release; within each group newest first
    versions.sort_by(|a, b| {
        b.is_prerelease
            .cmp(&a.is_prerelease)
            .then(b.version.cmp(&a.version))
    });

    Ok(web::Json(ExplorePackageDetailResponse {
        registry: registry.clone(),
        name: name.clone(),
        gate: GateDto {
            registry_accessible,
            beta_member,
        },
        versions,
        upstream_unavailable,
    }))
}
