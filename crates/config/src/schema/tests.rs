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

// ── Test helper: a minimal parseable AppConfig ────────────────────────────────

/// Parse a minimal `AppConfig` with `extra` appended verbatim.
///
/// `[server]` comes last so `extra` can start with bare `key = value` lines that
/// land in it, then open further tables (`[ip_blocking]`, `[[registries]]`, …).
///
/// Deliberately calls `toml::from_str` + `validate` rather than
/// `batlehub_config::load_from_str`, so a test can assert on validation errors
/// without env-var expansion or overrides in the way.
pub(super) fn parse_config(extra: &str) -> AppConfig {
    let raw = format!(
        r#"
        [database]
        type = "postgresql"
        url = "postgresql://localhost/test"

        [storage]
        type = "filesystem"
        path = "/tmp/batlehub-test"

        [server]
        host = "127.0.0.1"
        port = 8080
{extra}
        "#
    );
    toml::from_str(&raw).expect("test config parses")
}

// ── Proxy trust ───────────────────────────────────────────────────────────────

#[test]
fn trusted_proxies_absent_empty_and_populated_are_distinguishable() {
    assert!(parse_config("").server.trusted_proxies.is_none());
    assert_eq!(
        parse_config("        trusted_proxies = []")
            .server
            .trusted_proxies,
        Some(vec![])
    );
    assert_eq!(
        parse_config(r#"        trusted_proxies = ["10.42.0.0/16"]"#)
            .server
            .trusted_proxies,
        Some(vec!["10.42.0.0/16".to_owned()])
    );
}

#[test]
fn parse_trusted_proxies_accepts_cidr_and_bare_addresses() {
    let entries = vec![
        "10.42.0.0/16".to_owned(),
        "192.168.1.10".to_owned(),
        "2001:db8::/32".to_owned(),
        "::1".to_owned(),
    ];
    let nets = parse_trusted_proxies(&entries).expect("all entries valid");
    assert_eq!(nets.len(), 4);
    // A bare address is widened to its host prefix, so it still matches exactly one IP.
    assert_eq!(nets[1].prefix_len(), 32);
    assert_eq!(nets[3].prefix_len(), 128);
}

#[test]
fn parse_trusted_proxies_rejects_garbage() {
    let err = parse_trusted_proxies(&["not-an-ip".to_owned()]).unwrap_err();
    assert!(
        err.contains("not-an-ip"),
        "error names the bad entry: {err}"
    );
}

#[test]
fn validate_rejects_a_malformed_trusted_proxy_entry() {
    let err = parse_config(r#"        trusted_proxies = ["10.0.0.0/99"]"#)
        .validate()
        .unwrap_err()
        .to_string();
    assert!(err.contains("[server].trusted_proxies"), "{err}");
}

#[test]
fn a_malformed_deprecated_trusted_proxy_entry_warns_instead_of_failing_the_boot() {
    // The deprecated key predates any validator: entries it could not parse were
    // dropped. Rejecting one now would turn an upgrade into a CrashLoopBackOff
    // for a config file that never changed.
    let cfg = parse_config(
        r#"
        [ip_blocking]
        trusted_proxies = ["10.0.0.5", "ingress.internal"]"#,
    );
    cfg.validate()
        .expect("a stale entry must not fail the boot");

    let warning = cfg
        .warnings()
        .into_iter()
        .find(|w| w.code == warnings::PROXY_TRUST_INVALID_DEPRECATED_ENTRY)
        .expect("the dropped entry is surfaced as a warning");
    assert!(warning.message.contains("ingress.internal"), "{warning:?}");
}

#[test]
fn effective_trusted_proxies_prefers_server_over_the_deprecated_alias() {
    let cfg = parse_config(
        r#"        trusted_proxies = ["10.0.0.0/8"]

        [ip_blocking]
        trusted_proxies = ["192.0.2.1"]"#,
    );
    assert_eq!(
        cfg.effective_trusted_proxies(),
        Some(&["10.0.0.0/8".to_owned()][..])
    );
    assert!(cfg.shadows_deprecated_trusted_proxies());
    assert!(!cfg.uses_deprecated_trusted_proxies());
}

#[test]
fn effective_trusted_proxies_falls_back_to_the_deprecated_alias() {
    let cfg = parse_config(
        r#"
        [ip_blocking]
        trusted_proxies = ["192.0.2.1"]"#,
    );
    assert_eq!(
        cfg.effective_trusted_proxies(),
        Some(&["192.0.2.1".to_owned()][..])
    );
    assert!(cfg.uses_deprecated_trusted_proxies());
    assert!(!cfg.shadows_deprecated_trusted_proxies());
}

#[test]
fn an_empty_deprecated_list_is_not_a_policy_but_an_empty_server_list_is() {
    // `[ip_blocking].trusted_proxies` defaults to `[]`, so an empty value there
    // carries no operator intent — unlike `[server].trusted_proxies = []`, which
    // is an explicit "trust nobody".
    let cfg = parse_config(
        r#"
        [ip_blocking]
        enabled = true"#,
    );
    assert!(cfg.effective_trusted_proxies().is_none());

    let cfg = parse_config("        trusted_proxies = []");
    assert_eq!(cfg.effective_trusted_proxies(), Some(&[][..]));
}

// ── Config warnings ───────────────────────────────────────────────────────────

fn warning_codes(cfg: &AppConfig) -> Vec<String> {
    cfg.warnings().into_iter().map(|w| w.code).collect()
}

#[test]
fn a_config_with_no_proxy_trust_policy_warns_about_it() {
    let cfg = parse_config("");
    assert_eq!(
        warning_codes(&cfg),
        vec![warnings::PROXY_TRUST_UNCONFIGURED]
    );
    assert_eq!(cfg.warnings()[0].path, "server.trusted_proxies");
}

#[test]
fn an_explicit_empty_server_list_is_a_policy_and_does_not_warn() {
    assert!(parse_config("        trusted_proxies = []")
        .warnings()
        .is_empty());
}

#[test]
fn a_configured_server_list_does_not_warn() {
    assert!(parse_config(r#"        trusted_proxies = ["10.0.0.0/8"]"#)
        .warnings()
        .is_empty());
}

#[test]
fn the_deprecated_key_alone_warns_but_is_honoured() {
    let cfg = parse_config(
        r#"
        [ip_blocking]
        trusted_proxies = ["10.0.0.1"]"#,
    );
    assert_eq!(
        warning_codes(&cfg),
        vec![warnings::PROXY_TRUST_DEPRECATED_KEY_ONLY]
    );
    assert_eq!(cfg.warnings()[0].path, "ip_blocking.trusted_proxies");
    assert!(cfg.effective_trusted_proxies().is_some());
}

#[test]
fn a_shadowed_deprecated_key_warns_and_names_itself() {
    let cfg = parse_config(
        r#"        trusted_proxies = ["10.0.0.0/8"]

        [ip_blocking]
        trusted_proxies = ["10.0.0.1"]"#,
    );
    assert_eq!(
        warning_codes(&cfg),
        vec![warnings::PROXY_TRUST_SHADOWED_DEPRECATED_KEY]
    );
    assert_eq!(cfg.warnings()[0].path, "ip_blocking.trusted_proxies");
}

// ── Host-based routing (RFC 0001 §4.3) ────────────────────────────────────────

/// A config with proxy trust already declared, so host-routing tests assert on
/// the condition they are actually about.
fn host_routing_config(extra: &str) -> AppConfig {
    parse_config(&format!(
        "        trusted_proxies = [\"10.0.0.0/8\"]\n{extra}"
    ))
}

fn npm_registry(name: &str, extra: &str) -> String {
    format!(
        "
        [[registries]]
        type = \"npm\"
        name = \"{name}\"
{extra}"
    )
}

#[test]
fn registry_host_fields_default_to_no_hosts_and_path_routing_on() {
    let cfg = parse_config(&npm_registry("npm1", ""));
    assert!(cfg.registries[0].hosts.is_empty());
    assert!(cfg.registries[0].path_routing);
    assert!(!cfg.host_routing_configured());
    assert!(cfg.registry_host_bindings().is_empty());
}

#[test]
fn wildcard_and_explicit_hosts_both_bind_to_the_registry() {
    let cfg = host_routing_config(&format!(
        r#"
        [subdomain_routing]
        enabled = true
        base_domain = "hub.example.com"
{}"#,
        npm_registry("npm1", "        hosts = [\"NPM.Acme.io\"]")
    ));
    cfg.validate().expect("valid");
    let bindings = cfg.registry_host_bindings();
    assert_eq!(bindings.len(), 2);
    // Explicit entries come first and are normalised.
    assert_eq!(bindings[0].host, "npm.acme.io");
    assert!(bindings[0].explicit);
    assert_eq!(bindings[1].host, "npm1.hub.example.com");
    assert!(!bindings[1].explicit);
    // The explicit host is the advertised one.
    assert_eq!(
        cfg.registry_public_urls(),
        vec![("npm1".to_owned(), "https://npm.acme.io".to_owned())]
    );
}

#[test]
fn public_url_falls_back_to_the_wildcard_host_and_honours_scheme() {
    let cfg = host_routing_config(&format!(
        r#"
        [subdomain_routing]
        enabled = true
        base_domain = "hub.example.com"
        scheme = "http"
{}"#,
        npm_registry("npm1", "")
    ));
    assert_eq!(
        cfg.registry_public_urls(),
        vec![("npm1".to_owned(), "http://npm1.hub.example.com".to_owned())]
    );
}

#[test]
fn enabled_without_base_domain_is_rejected() {
    let err = host_routing_config(
        r#"
        [subdomain_routing]
        enabled = true"#,
    )
    .validate()
    .unwrap_err()
    .to_string();
    assert!(err.contains("base_domain"), "{err}");
}

#[test]
fn a_url_shaped_base_domain_is_rejected() {
    let err = host_routing_config(
        r#"
        [subdomain_routing]
        enabled = true
        base_domain = "https://hub.example.com""#,
    )
    .validate()
    .unwrap_err()
    .to_string();
    assert!(err.contains("base_domain"), "{err}");
    assert!(err.contains("scheme prefix"), "{err}");
}

#[test]
fn a_base_domain_with_a_path_is_rejected() {
    let err = host_routing_config(
        r#"
        [subdomain_routing]
        enabled = true
        base_domain = "hub.example.com/registry""#,
    )
    .validate()
    .unwrap_err()
    .to_string();
    assert!(err.contains("base_domain"), "{err}");
}

#[test]
fn two_registries_claiming_the_same_host_is_rejected() {
    let cfg = host_routing_config(&format!(
        "{}{}",
        npm_registry("npm1", "        hosts = [\"npm.acme.io\"]"),
        npm_registry("npm2", "        hosts = [\"NPM.acme.io.\"]")
    ));
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("npm.acme.io"), "{err}");
    assert!(err.contains("npm1") && err.contains("npm2"), "{err}");
}

#[test]
fn an_explicit_host_colliding_with_another_registrys_wildcard_is_rejected() {
    let cfg = host_routing_config(&format!(
        r#"
        [subdomain_routing]
        enabled = true
        base_domain = "hub.example.com"
{}{}"#,
        npm_registry("npm1", ""),
        npm_registry("npm2", "        hosts = [\"npm1.hub.example.com\"]")
    ));
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("npm1.hub.example.com"), "{err}");
}

#[test]
fn a_registry_may_restate_its_own_wildcard_host_explicitly() {
    let cfg = host_routing_config(&format!(
        r#"
        [subdomain_routing]
        enabled = true
        base_domain = "hub.example.com"
{}"#,
        npm_registry("npm1", "        hosts = [\"npm1.hub.example.com\"]")
    ));
    cfg.validate()
        .expect("same-registry duplicate is not a conflict");
}

#[test]
fn a_host_equal_to_the_base_domain_is_rejected() {
    let cfg = host_routing_config(&format!(
        r#"
        [subdomain_routing]
        enabled = true
        base_domain = "hub.example.com"
{}"#,
        npm_registry("npm1", "        hosts = [\"hub.example.com\"]")
    ));
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("admin API"), "{err}");
}

#[test]
fn malformed_host_entries_are_rejected() {
    for (entry, needle) in [
        ("https://npm.acme.io", "scheme prefix"),
        ("npm.acme.io/proxy", "contains '/'"),
        ("   ", "empty"),
    ] {
        let cfg = host_routing_config(&npm_registry(
            "npm1",
            &format!("        hosts = [\"{entry}\"]"),
        ));
        let err = cfg.validate().unwrap_err().to_string();
        assert!(err.contains(needle), "entry {entry:?} → {err}");
    }
}

#[test]
fn path_routing_false_without_a_reachable_host_is_rejected() {
    let cfg = host_routing_config(&format!(
        "{}{}",
        npm_registry("npm1", "        hosts = [\"npm.acme.io\"]"),
        npm_registry("npm2", "        path_routing = false")
    ));
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("npm2") && err.contains("no ingress"), "{err}");
}

#[test]
fn path_routing_false_with_a_host_is_accepted() {
    let cfg = host_routing_config(&npm_registry(
        "npm1",
        "        hosts = [\"npm.acme.io\"]\n        path_routing = false",
    ));
    cfg.validate().expect("valid");
    assert_eq!(cfg.host_only_registries(), vec!["npm1".to_owned()]);
}

#[test]
fn path_routing_false_satisfied_by_a_wildcard_host_is_accepted() {
    let cfg = host_routing_config(&format!(
        r#"
        [subdomain_routing]
        enabled = true
        base_domain = "hub.example.com"
{}"#,
        npm_registry("npm1", "        path_routing = false")
    ));
    cfg.validate().expect("wildcard host is a reachable host");
}

#[test]
fn host_routing_without_any_trusted_proxy_policy_is_rejected() {
    let err = parse_config(&npm_registry("npm1", "        hosts = [\"npm.acme.io\"]"))
        .validate()
        .unwrap_err()
        .to_string();
    assert!(err.contains("trusted_proxies"), "{err}");
    // The message contains the exact TOML to paste.
    assert!(err.contains("[server]"), "{err}");
}

#[test]
fn host_routing_is_accepted_when_only_the_deprecated_key_is_set_and_it_warns() {
    let cfg = parse_config(&format!(
        r#"{}
        [ip_blocking]
        trusted_proxies = ["10.0.0.1"]"#,
        npm_registry("npm1", "        hosts = [\"npm.acme.io\"]")
    ));
    cfg.validate()
        .expect("the deprecated alias satisfies the requirement");
    assert_eq!(
        warning_codes(&cfg),
        vec![warnings::PROXY_TRUST_DEPRECATED_KEY_ONLY]
    );
}

#[test]
fn an_empty_server_list_satisfies_the_host_routing_requirement() {
    // "trust nobody" is a stated policy: BatleHub is exposed directly and only
    // the `Host` header routes.
    parse_config(&format!(
        "        trusted_proxies = []\n{}",
        npm_registry("npm1", "        hosts = [\"npm.acme.io\"]")
    ))
    .validate()
    .expect("an explicit empty list is a policy");
}

#[test]
fn a_non_dns_label_registry_name_warns_and_derives_no_wildcard() {
    let cfg = host_routing_config(&format!(
        r#"
        [subdomain_routing]
        enabled = true
        base_domain = "hub.example.com"
{}{}"#,
        npm_registry("my_registry", ""),
        npm_registry("npm1", "")
    ));
    cfg.validate()
        .expect("valid — this degrades, it does not fail");

    let w = cfg.warnings();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].code, warnings::SUBDOMAIN_INVALID_DNS_LABEL);
    assert_eq!(w[0].path, "registries[0].name");

    // Only the well-named registry gets a wildcard host.
    let hosts: Vec<String> = cfg
        .registry_host_bindings()
        .into_iter()
        .map(|b| b.host)
        .collect();
    assert_eq!(hosts, vec!["npm1.hub.example.com".to_owned()]);
}

#[test]
fn no_dns_label_warning_when_wildcard_derivation_is_off() {
    let cfg = parse_config(&npm_registry("my_registry", ""));
    assert!(!warning_codes(&cfg).contains(&warnings::SUBDOMAIN_INVALID_DNS_LABEL.to_owned()));
}
