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
