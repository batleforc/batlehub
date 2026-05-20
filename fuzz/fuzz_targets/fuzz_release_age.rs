#![no_main]

use std::sync::OnceLock;
use std::time::Duration;

use libfuzzer_sys::fuzz_target;
use tokio::runtime::Runtime;

use batlehub_core::{
    entities::{Identity, PackageId, PackageMetadata, Role},
    rules::{ReleaseAgeGateRule, Rule, RuleContext},
};

static RT: OnceLock<Runtime> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let rt = RT.get_or_init(|| Runtime::new().unwrap());
    let mut u = arbitrary::Unstructured::new(data);

    let Ok(timestamp_secs): arbitrary::Result<i64> = u.arbitrary() else { return };
    let Ok(min_age_secs): arbitrary::Result<u64> = u.arbitrary() else { return };
    let Ok(role_idx): arbitrary::Result<u8> = u.arbitrary() else { return };

    let role = match role_idx % 3 {
        0 => Role::Anonymous,
        1 => Role::User,
        _ => Role::Admin,
    };

    let Some(published_at) = chrono::DateTime::from_timestamp(timestamp_secs, 0) else {
        return;
    };

    // Cap at one year to keep durations meaningful.
    let min_age = Duration::from_secs(min_age_secs.min(365 * 24 * 3600));

    let rule = ReleaseAgeGateRule::new(min_age, vec![Role::Admin]);

    let identity = Identity { user_id: None, role, auth_provider: None, groups: vec![] };
    let meta = PackageMetadata {
        id: PackageId::new("github", "owner/repo", "v1.0.0"),
        published_at: Some(published_at),
        download_url: None,
        checksum: None,
        is_signed: None,
        extra: serde_json::Value::Null,
    };
    let ctx = RuleContext {
        identity: &identity,
        package: &meta,
        resource_type: "releases:read",
        cache_entry: None,
        requested_version: None,
    };

    let _ = rt.block_on(rule.evaluate(&ctx));
});
