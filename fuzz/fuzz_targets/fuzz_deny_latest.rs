#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use tokio::runtime::Runtime;

use proxy_cache_core::{
    entities::{Identity, PackageId, PackageMetadata, Role},
    rules::{DenyLatestRule, Rule, RuleContext, RuleDecision},
};

static RT: OnceLock<Runtime> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let rt = RT.get_or_init(|| Runtime::new().unwrap());
    let mut u = arbitrary::Unstructured::new(data);

    let Ok(version_str): arbitrary::Result<String> = u.arbitrary() else { return };
    let Ok(role_idx): arbitrary::Result<u8> = u.arbitrary() else { return };

    let role = match role_idx % 3 {
        0 => Role::Anonymous,
        1 => Role::User,
        _ => Role::Admin,
    };

    // Bypass list includes Admin to exercise the RequireRole path.
    let rule = DenyLatestRule::new(vec![Role::Admin]);

    let identity = Identity { user_id: None, role, auth_provider: None, groups: vec![] };
    let meta = PackageMetadata {
        id: PackageId::new("npm", "pkg", &version_str),
        published_at: None,
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
        requested_version: Some(&version_str),
    };

    let decision = rt.block_on(rule.evaluate(&ctx));

    // Security invariant: only the exact string "latest" triggers a deny.
    // Unicode homoglyphs or whitespace variations must NOT bypass the deny
    // and must NOT accidentally block legitimate version strings.
    if version_str == "latest" {
        assert!(decision.is_deny(), "\"latest\" must always be blocked");
    } else {
        assert!(
            matches!(decision, RuleDecision::Allow),
            "non-\"latest\" version {:?} must always be allowed",
            version_str
        );
    }
});
