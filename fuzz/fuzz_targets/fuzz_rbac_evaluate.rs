#![no_main]

use std::collections::HashMap;
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use tokio::runtime::Runtime;

use batlehub_core::{
    entities::{Identity, PackageId, PackageMetadata, Role},
    rules::{RbacRule, Rule, RuleContext},
};

static RT: OnceLock<Runtime> = OnceLock::new();

fuzz_target!(|data: &[u8]| {
    let rt = RT.get_or_init(|| Runtime::new().unwrap());
    let mut u = arbitrary::Unstructured::new(data);

    let Ok(groups): arbitrary::Result<Vec<String>> = u.arbitrary() else { return };
    let Ok(resource_type): arbitrary::Result<String> = u.arbitrary() else { return };
    let Ok(role_idx): arbitrary::Result<u8> = u.arbitrary() else { return };

    let role = match role_idx % 3 {
        0 => Role::Anonymous,
        1 => Role::User,
        _ => Role::Admin,
    };

    let rule = RbacRule::new(HashMap::from([
        (Role::Anonymous, vec!["releases:read".to_owned()]),
        (Role::User, vec!["releases:read".to_owned(), "source:read".to_owned()]),
        (Role::Admin, vec!["*".to_owned()]),
    ]))
    .with_groups(HashMap::from([
        ("*:team-a".to_owned(), vec!["releases:read".to_owned()]),
        ("oidc:team-b".to_owned(), vec!["source:read".to_owned()]),
    ]));

    let identity = Identity { user_id: None, role, auth_provider: None, groups };
    let meta = PackageMetadata {
        id: PackageId::new("test", "pkg", "1.0"),
        published_at: None,
        download_url: None,
        checksum: None,
        is_signed: None,
        extra: serde_json::Value::Null,
    };
    let ctx = RuleContext {
        identity: &identity,
        package: &meta,
        resource_type: &resource_type,
        cache_entry: None,
        requested_version: None,
    };

    let _ = rt.block_on(rule.evaluate(&ctx));
});
