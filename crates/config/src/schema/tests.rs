use super::*;

#[test]
fn cache_policy_defaults() {
    let p: CachePolicy = toml::from_str("").unwrap();
    assert_eq!(p.metadata_ttl_secs, 300);
    assert!(p.serve_stale);
    assert!(p.artifact_ttl_secs.is_none());
    assert!(p.idle_days.is_none());
    assert!(p.max_size_bytes.is_none());
    assert!(p.keep_latest_n.is_none());
}

#[test]
fn cache_policy_full_config() {
    let raw = r#"
        metadata_ttl_secs = 60
        serve_stale = false
        artifact_ttl_secs = 3600
        idle_days = 30
        max_size_bytes = 10000000
        keep_latest_n = 5
    "#;
    let p: CachePolicy = toml::from_str(raw).unwrap();
    assert_eq!(p.metadata_ttl_secs, 60);
    assert!(!p.serve_stale);
    assert_eq!(p.artifact_ttl_secs, Some(3600));
    assert_eq!(p.idle_days, Some(30));
    assert_eq!(p.max_size_bytes, Some(10_000_000));
    assert_eq!(p.keep_latest_n, Some(5));
}

#[test]
fn cache_policy_partial_config_uses_defaults_for_unset_fields() {
    let raw = "artifact_ttl_secs = 7200";
    let p: CachePolicy = toml::from_str(raw).unwrap();
    assert_eq!(
        p.metadata_ttl_secs, 300,
        "metadata_ttl_secs should use default"
    );
    assert!(p.serve_stale, "serve_stale should default to true");
    assert_eq!(p.artifact_ttl_secs, Some(7200));
    assert!(p.idle_days.is_none());
    assert!(p.max_size_bytes.is_none());
    assert!(p.keep_latest_n.is_none());
}

#[test]
fn cache_policy_zero_keep_latest_n_is_valid() {
    let raw = "keep_latest_n = 1";
    let p: CachePolicy = toml::from_str(raw).unwrap();
    assert_eq!(p.keep_latest_n, Some(1));
}

#[test]
fn cache_policy_default_impl_matches_toml_defaults() {
    let from_default = CachePolicy::default();
    let from_toml: CachePolicy = toml::from_str("").unwrap();
    assert_eq!(from_default.metadata_ttl_secs, from_toml.metadata_ttl_secs);
    assert_eq!(from_default.serve_stale, from_toml.serve_stale);
    assert_eq!(from_default.artifact_ttl_secs, from_toml.artifact_ttl_secs);
    assert_eq!(from_default.idle_days, from_toml.idle_days);
    assert_eq!(from_default.max_size_bytes, from_toml.max_size_bytes);
    assert_eq!(from_default.keep_latest_n, from_toml.keep_latest_n);
}

// ── Feature flags ─────────────────────────────────────────────────────────────

#[test]
fn feature_flags_default_socket_badge_on() {
    let f: FeatureFlagsConfig = toml::from_str("").unwrap();
    assert!(f.socket_badge, "socket_badge defaults to true");
    assert!(FeatureFlagsConfig::default().socket_badge);
}

#[test]
fn feature_flags_can_disable_socket_badge() {
    let f: FeatureFlagsConfig = toml::from_str("socket_badge = false").unwrap();
    assert!(!f.socket_badge);
}

#[test]
fn integrity_defaults_verify_and_block_on_mismatch() {
    // An empty (or partial) block must fall back to verify + block-on-mismatch.
    let i: IntegrityConfig = toml::from_str("").unwrap();
    assert!(i.enabled);
    assert!(i.block_on_mismatch);
    assert!(!i.require_metadata);
    assert!(i.bypass_roles.is_empty());

    let d = IntegrityConfig::default();
    assert!(d.enabled);
    assert!(d.block_on_mismatch);
    assert!(!d.require_metadata);
}

#[test]
fn integrity_parses_full_block() {
    let raw = r#"
        type = "cargo"
        name = "crates"
        [integrity]
        enabled = true
        block_on_mismatch = false
        require_metadata = true
        bypass_roles = ["admin"]
    "#;
    let reg: RegistryConfig = toml::from_str(raw).unwrap();
    let i = reg.integrity.expect("integrity block parsed");
    assert!(i.enabled);
    assert!(!i.block_on_mismatch);
    assert!(i.require_metadata);
    assert_eq!(i.bypass_roles, vec!["admin".to_owned()]);
}

#[test]
fn registry_parses_feature_flags_block() {
    let raw = r#"
        type = "cargo"
        name = "crates"
        [feature_flags]
        socket_badge = false
    "#;
    let reg: RegistryConfig = toml::from_str(raw).unwrap();
    assert!(!reg.feature_flags.unwrap().socket_badge);
}

// ── CVE gate rule ─────────────────────────────────────────────────────────────

#[test]
fn cve_gate_rule_parses_with_defaults() {
    let raw = r#"kind = "cve_gate""#;
    let rule: RuleConfig = toml::from_str(raw).unwrap();
    match rule {
        RuleConfig::CveGate(c) => {
            assert_eq!(c.min_severity, "high");
            assert!(!c.block);
            assert!(c.bypass_roles.is_empty());
        }
        other => panic!("expected CveGate, got {other:?}"),
    }
}

#[test]
fn cve_gate_rule_parses_full() {
    let raw = r#"
        kind = "cve_gate"
        min_severity = "critical"
        block = true
        bypass_roles = ["admin"]
    "#;
    let rule: RuleConfig = toml::from_str(raw).unwrap();
    match rule {
        RuleConfig::CveGate(c) => {
            assert_eq!(c.min_severity, "critical");
            assert!(c.block);
            assert_eq!(c.bypass_roles, vec!["admin".to_owned()]);
        }
        other => panic!("expected CveGate, got {other:?}"),
    }
}

// ── Trusted publisher rule ────────────────────────────────────────────────────

#[test]
fn trusted_publisher_rule_parses_with_defaults() {
    let raw = r#"kind = "trusted_publisher""#;
    let rule: RuleConfig = toml::from_str(raw).unwrap();
    match rule {
        RuleConfig::TrustedPublisher(c) => {
            assert!(c.allow.is_empty());
            assert!(c.bypass_roles.is_empty());
        }
        other => panic!("expected TrustedPublisher, got {other:?}"),
    }
}

#[test]
fn trusted_publisher_rule_parses_full() {
    let raw = r#"
        kind = "trusted_publisher"
        allow = ["my-org", "trusted-user"]
        bypass_roles = ["admin"]
    "#;
    let rule: RuleConfig = toml::from_str(raw).unwrap();
    match rule {
        RuleConfig::TrustedPublisher(c) => {
            assert_eq!(
                c.allow,
                vec!["my-org".to_owned(), "trusted-user".to_owned()]
            );
            assert_eq!(c.bypass_roles, vec!["admin".to_owned()]);
        }
        other => panic!("expected TrustedPublisher, got {other:?}"),
    }
}

// ── Vulnerability scan ────────────────────────────────────────────────────────

#[test]
fn vulnerability_scan_defaults() {
    let v: VulnerabilityScanConfig = toml::from_str("enabled = true").unwrap();
    assert!(v.enabled);
    assert_eq!(v.interval_secs, 86_400);
    assert_eq!(v.batch_size, 100);
    assert!(v.osv_api_url.is_none());
}

#[test]
fn vulnerability_scan_full() {
    let raw = r#"
        enabled = true
        interval_secs = 3600
        osv_api_url = "https://osv.local"
        batch_size = 25
    "#;
    let v: VulnerabilityScanConfig = toml::from_str(raw).unwrap();
    assert_eq!(v.interval_secs, 3600);
    assert_eq!(v.osv_api_url.as_deref(), Some("https://osv.local"));
    assert_eq!(v.batch_size, 25);
}
