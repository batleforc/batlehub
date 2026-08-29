#![no_main]

use std::collections::HashMap;
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use tokio::runtime::Runtime;

use batlehub_core::{
    entities::Action,
    entities::{Identity, PackageId, PackageMetadata, Role},
    rules::{RbacRule, Rule, RuleContext},
};

static RT: OnceLock<Runtime> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let rt = RT.get_or_init(|| Runtime::new().unwrap());
    let mut u = arbitrary::Unstructured::new(data);

    let Ok(groups): arbitrary::Result<Vec<String>> = u.arbitrary() else { return };
    // The fuzzed *string* moved. Until RFC 0015 phase 1 an arbitrary
    // `resource_type` reached `evaluate` directly, and that was the surface
    // worth fuzzing; a closed `Action` makes it unrepresentable there. The
    // arbitrary strings now enter one layer earlier — as config patterns handed
    // to `from_patterns`, which parses and expands them — so that is what this
    // target feeds, and the verb under evaluation is picked from the enum.
    let Ok(patterns): arbitrary::Result<Vec<String>> = u.arbitrary() else { return };
    let Ok(action_idx): arbitrary::Result<u8> = u.arbitrary() else { return };
    let Ok(role_idx): arbitrary::Result<u8> = u.arbitrary() else { return };

    let action = Action::ALL[action_idx as usize % Action::ALL.len()];

    let role = match role_idx % 3 {
        0 => Role::Anonymous,
        1 => Role::User,
        _ => Role::Admin,
    };

    // A malformed pattern is a config-load error, not a panic and not a silent
    // grant — which is the property under test. Returning on `Err` is the
    // assertion: reaching `evaluate` at all means the patterns parsed.
    let Ok(rule) = RbacRule::from_patterns(HashMap::from([
        (Role::Anonymous, vec!["releases:read".to_owned()]),
        (Role::User, patterns.clone()),
        (Role::Admin, vec!["*".to_owned()]),
    ])) else { return };
    let Ok(rule) = rule.with_group_patterns(HashMap::from([
        ("*:team-a".to_owned(), vec!["releases:read".to_owned()]),
        ("oidc:team-b".to_owned(), patterns),
    ])) else { return };

    let identity = Identity { user_id: None, role, auth_provider: None, groups };
    let meta = PackageMetadata {
        id: PackageId::new("test", "pkg", "1.0"),
        published_at: None,
        download_url: None,
        checksum: None,
        is_signed: None,
        extra: serde_json::Value::Null,
        // Not read by the rule under test — it carries an upstream
        // `Cache-Control`, and this fuzzer is about the decision, not the cache.
        // Written out rather than `..Default::default()` on purpose: an exhaustive
        // literal is what makes a new field on `PackageMetadata` stop this target
        // compiling, and the CI job that builds these bins is what turns that into
        // a prompt rather than silent non-coverage.
        cache_control: None,
    };
    let ctx = RuleContext {
        identity: &identity,
        package: &meta,
        action,
        cache_entry: None,
        requested_version: None,
    };

    let _ = rt.block_on(rule.evaluate(&ctx));
});
