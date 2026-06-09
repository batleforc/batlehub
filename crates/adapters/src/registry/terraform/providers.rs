use super::{log, models, percent_encode, TerraformRegistryClient, UpstreamPackage};
use models::{ModuleSearch, ProviderList};

/// Search providers: namespace listing + optional exact lookup.
///
/// Returns up to `per` results from:
/// 1. `GET /v1/providers/{query}` — all providers in that namespace
/// 2. `GET /v1/providers/{ns}/{type}/versions` — exact namespace/type if query contains `/`
pub(super) async fn search_providers(
    client: &TerraformRegistryClient,
    base: &str,
    query: &str,
    per: usize,
) -> Vec<UpstreamPackage> {
    let namespace_url = format!("{}/v1/providers/{}", base, percent_encode(query));
    let exact_url = query.split_once('/').map(|(ns, ty)| {
        format!(
            "{}/v1/providers/{}/{}/versions",
            base,
            percent_encode(ns),
            percent_encode(ty),
        )
    });

    let mut results: Vec<UpstreamPackage> = Vec::new();

    // Provider namespace listing
    if let Some(body) = client
        .fetch_json::<ProviderList>(&namespace_url, "tf provider ns")
        .await
    {
        log::debug!(count = body.providers.len(), "tf provider ns: ok");
        for p in body.providers.into_iter().take(per) {
            results.push(UpstreamPackage {
                name: format!("providers/{}/{}", p.namespace, p.name),
                latest_version: p.version.unwrap_or_else(|| "latest".to_string()),
                description: p.description,
            });
        }
    }

    // Exact namespace/type provider lookup
    if let Some(url) = exact_url {
        if let Some(body) = client
            .fetch_json::<ProviderList>(&url, "tf provider exact")
            .await
        {
            for p in body.providers.into_iter().take(per) {
                results.push(UpstreamPackage {
                    name: format!("providers/{}/{}", p.namespace, p.name),
                    latest_version: p.version.unwrap_or_else(|| "latest".to_string()),
                    description: p.description,
                });
            }
        }
    }

    results
}

/// Search modules via `GET /v1/modules/search?q=...`
pub(super) async fn search_modules(
    client: &TerraformRegistryClient,
    base: &str,
    query: &str,
    per: usize,
) -> Vec<UpstreamPackage> {
    let module_url = format!(
        "{}/v1/modules/search?q={}&limit={}",
        base,
        percent_encode(query),
        per,
    );

    let mut results = Vec::new();
    if let Some(body) = client
        .fetch_json::<ModuleSearch>(&module_url, "tf module search")
        .await
    {
        log::debug!(count = body.modules.len(), "tf module search: ok");
        for m in body.modules.into_iter().take(per) {
            results.push(UpstreamPackage {
                name: format!("modules/{}/{}/{}", m.namespace, m.name, m.provider),
                latest_version: m.version,
                description: m.description,
            });
        }
    }
    results
}
