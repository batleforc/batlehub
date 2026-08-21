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

// ── Licence gate rule ─────────────────────────────────────────────────────────

/// The defaults are the whole safety story: warn-only, and an unknown licence
/// is permitted. A `license_gate` added to a config must not start refusing
/// traffic the moment it is written (RFC 0004-bis §13.1).
#[test]
fn license_gate_rule_parses_with_defaults() {
    let raw = r#"kind = "license_gate""#;
    let rule: RuleConfig = toml::from_str(raw).unwrap();
    match rule {
        RuleConfig::LicenseGate(c) => {
            assert!(c.allow.is_empty());
            assert!(c.deny.is_empty());
            assert!(
                c.allow_unknown,
                "an unknown licence must not deny by default"
            );
            assert!(!c.block, "the gate must be warn-only until asked otherwise");
            assert!(c.bypass_roles.is_empty());
        }
        other => panic!("expected LicenseGate, got {other:?}"),
    }
}

#[test]
fn license_gate_rule_parses_full() {
    let raw = r#"
        kind = "license_gate"
        allow = ["MIT", "Apache-2.0"]
        deny = ["AGPL-3.0"]
        allow_unknown = false
        block = true
        bypass_roles = ["admin"]
    "#;
    let rule: RuleConfig = toml::from_str(raw).unwrap();
    match rule {
        RuleConfig::LicenseGate(c) => {
            assert_eq!(c.allow, vec!["MIT".to_owned(), "Apache-2.0".to_owned()]);
            assert_eq!(c.deny, vec!["AGPL-3.0".to_owned()]);
            assert!(!c.allow_unknown);
            assert!(c.block);
            assert_eq!(c.bypass_roles, vec!["admin".to_owned()]);
        }
        other => panic!("expected LicenseGate, got {other:?}"),
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

// ── Licence gate warnings ─────────────────────────────────────────────────────

/// Base config plus one registry carrying a `license_gate` rule.
fn config_with_license_gate(registry_type: &str, rule_body: &str) -> AppConfig {
    parse_config(&format!(
        r#"
        [[registries]]
        type = "{registry_type}"
        name = "reg"

        [registries.sbom]
        enabled = true

        [[registries.rules]]
        kind = "license_gate"
{rule_body}"#
    ))
}

/// A registry type with a manifest parser is the case the rule was written for,
/// and it must stay quiet — a warning that fires on correct config is noise
/// that teaches operators to ignore the channel.
#[test]
fn license_gate_on_a_supported_type_does_not_warn() {
    let cfg = config_with_license_gate("npm", r#"        allow = ["MIT"]"#);
    assert!(!warning_codes(&cfg)
        .iter()
        .any(|c| c.starts_with("license-gate")));
}

/// Sixteen of the twenty-one registry types have no manifest parser, so the
/// licence is permanently unknown and — with the default `allow_unknown = true`
/// — the rule never denies anything. Nothing errors at runtime, which is
/// exactly why it has to be said here (RFC 0004-bis §13.1).
#[test]
fn license_gate_on_a_type_with_no_parser_warns_that_it_never_fires() {
    let cfg = config_with_license_gate("goproxy", r#"        allow = ["MIT"]"#);
    assert!(warning_codes(&cfg).contains(&warnings::LICENSE_GATE_NO_EXTRACTOR.to_owned()));

    let w = cfg
        .warnings()
        .into_iter()
        .find(|w| w.code == warnings::LICENSE_GATE_NO_EXTRACTOR)
        .expect("warning emitted");
    assert_eq!(w.path, "registries[0].rules[0]");
    assert!(w.message.contains("never denies"), "{}", w.message);
    // The message names what *is* covered, so the operator can act on it.
    assert!(w.message.contains("npm"), "{}", w.message);
}

/// The opposite consequence from the same missing parser, and the one an
/// operator will be triaging under pressure: every download refused. It gets
/// its own code so it can be found by name.
#[test]
fn license_gate_that_refuses_everything_gets_its_own_code() {
    let cfg = config_with_license_gate(
        "goproxy",
        "        block = true\n        allow_unknown = false",
    );
    let codes = warning_codes(&cfg);
    assert!(codes.contains(&warnings::LICENSE_GATE_DENIES_EVERYTHING.to_owned()));
    // Not both: the two states are mutually exclusive, and emitting each would
    // make the dangerous one harder to see.
    assert!(!codes.contains(&warnings::LICENSE_GATE_NO_EXTRACTOR.to_owned()));
}

/// `allow_unknown = false` without `block` is still warn-only, so nothing is
/// refused — it is the inert case, not the dangerous one.
#[test]
fn allow_unknown_false_without_block_is_still_only_inert() {
    let cfg = config_with_license_gate("goproxy", "        allow_unknown = false");
    let codes = warning_codes(&cfg);
    assert!(codes.contains(&warnings::LICENSE_GATE_NO_EXTRACTOR.to_owned()));
    assert!(!codes.contains(&warnings::LICENSE_GATE_DENIES_EVERYTHING.to_owned()));
}

/// A registry with no `license_gate` has nothing to say, whatever its type.
#[test]
fn a_registry_without_a_license_gate_never_warns_about_one() {
    let cfg = parse_config(
        r#"
        [[registries]]
        type = "goproxy"
        name = "reg""#,
    );
    assert!(!warning_codes(&cfg)
        .iter()
        .any(|c| c.starts_with("license-gate")));
}

/// The licence is a side effect of SBOM generation, so a `license_gate` on a
/// registry with SBOM off never sees anything — even on a registry type whose
/// parser works perfectly. This was found by running the server, not by reading
/// it: extraction was correct, the rule was loaded, and no licence was ever
/// stored because `maybe_trigger_sbom` returned early.
#[test]
fn license_gate_without_sbom_enabled_warns_that_nothing_is_extracted() {
    let cfg = parse_config(
        r#"
        [[registries]]
        type = "npm"
        name = "reg"

        [[registries.rules]]
        kind = "license_gate"
        deny = ["AGPL-3.0"]"#,
    );
    let codes = warning_codes(&cfg);
    assert!(codes.contains(&warnings::LICENSE_GATE_SBOM_DISABLED.to_owned()));

    let w = cfg
        .warnings()
        .into_iter()
        .find(|w| w.code == warnings::LICENSE_GATE_SBOM_DISABLED)
        .expect("warning emitted");
    assert!(w.message.contains("never deny anything"), "{}", w.message);
    assert!(w.message.contains("[registries.sbom]"), "{}", w.message);
}

/// `enabled = false` is the same state as an absent block, and is the easier
/// one to overlook because the block is *there*.
#[test]
fn license_gate_with_sbom_explicitly_disabled_warns_too() {
    let cfg = parse_config(
        r#"
        [[registries]]
        type = "npm"
        name = "reg"

        [registries.sbom]
        enabled = false

        [[registries.rules]]
        kind = "license_gate"
        deny = ["AGPL-3.0"]"#,
    );
    assert!(warning_codes(&cfg).contains(&warnings::LICENSE_GATE_SBOM_DISABLED.to_owned()));
}

/// SBOM off *and* block + allow_unknown = false is the dangerous combination:
/// nothing is ever extracted, so every download is refused. The message has to
/// say that rather than the generic "never denies".
#[test]
fn sbom_disabled_with_strict_unknown_says_it_refuses_everything() {
    let cfg = parse_config(
        r#"
        [[registries]]
        type = "npm"
        name = "reg"

        [[registries.rules]]
        kind = "license_gate"
        block = true
        allow_unknown = false"#,
    );
    let w = cfg
        .warnings()
        .into_iter()
        .find(|w| w.code == warnings::LICENSE_GATE_SBOM_DISABLED)
        .expect("warning emitted");
    assert!(w.message.contains("refuse every download"), "{}", w.message);
}

/// One warning per rule, not two: SBOM being off already makes the licence
/// unknown, so also reporting the missing parser would be the same fact twice
/// and would bury whichever one the operator can act on.
#[test]
fn sbom_disabled_on_an_unsupported_type_reports_only_the_sbom_problem() {
    let cfg = parse_config(
        r#"
        [[registries]]
        type = "goproxy"
        name = "reg"

        [[registries.rules]]
        kind = "license_gate"
        deny = ["AGPL-3.0"]"#,
    );
    let codes = warning_codes(&cfg);
    assert!(codes.contains(&warnings::LICENSE_GATE_SBOM_DISABLED.to_owned()));
    assert!(!codes.contains(&warnings::LICENSE_GATE_NO_EXTRACTOR.to_owned()));
}

// ── README capture (RFC 0007 §4.5) ────────────────────────────────────────────

fn config_with_readme(registry_type: &str, extra: &str, block: &str) -> AppConfig {
    parse_config(&format!(
        r#"
        [[registries]]
        type = "{registry_type}"
        name = "reg"
        upstreams = ["https://example.invalid"]
{extra}
        [registries.readme]
{block}"#
    ))
}

/// An unrecognised `remote_images` must not silently become the default: the
/// two behaviours differ in what leaves the network, so an operator who wrote
/// `"allow"` expecting images believes the opposite of what they would get.
#[test]
fn an_unknown_remote_images_value_refuses_to_start() {
    let cfg = config_with_readme("npm", "", r#"        remote_images = "allow""#);
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("remote_images"), "{err}");
    assert!(err.contains("there is no \"allow\""), "{err}");
}

#[test]
fn the_two_documented_remote_images_values_are_accepted() {
    for value in ["strip", "proxy"] {
        let cfg = config_with_readme("npm", "", &format!("        remote_images = \"{value}\""));
        cfg.validate().expect("documented value accepted");
    }
}

/// Stores nothing while claiming to be on. A configuration that cannot do its
/// job should not start.
#[test]
fn max_bytes_zero_while_enabled_refuses_to_start() {
    let cfg = config_with_readme("npm", "", "        max_bytes = 0");
    assert!(cfg
        .validate()
        .unwrap_err()
        .to_string()
        .contains("max_bytes"));
}

/// With the feature off, a zero cap says nothing and is not worth refusing over.
#[test]
fn max_bytes_zero_with_the_feature_off_is_not_an_error() {
    let cfg = config_with_readme("npm", "", "        enabled = false\n        max_bytes = 0");
    cfg.validate()
        .expect("disabled registry stores nothing anyway");
}

#[test]
fn max_bytes_above_the_ceiling_refuses_to_start() {
    let cfg = config_with_readme("npm", "", "        max_bytes = 8388608");
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("ceiling"), "{err}");
}

/// The block written down on a type that has no README is accepted and inert,
/// and says so by name — with the reason `readme_support()` carries, so the
/// warning cannot drift from the behaviour.
#[test]
fn a_readme_block_on_a_type_with_none_warns_that_it_is_inert() {
    let cfg = config_with_readme("maven", "", "        enabled = true");
    let w = cfg
        .warnings()
        .into_iter()
        .find(|w| w.code == warnings::README_UNSUPPORTED_TYPE)
        .expect("warning emitted");
    assert!(w.message.contains("maven"), "{}", w.message);
    assert!(w.message.contains("sentence"), "{}", w.message);
    assert_eq!(w.path, "registries[0].readme");
}

/// The feature is on by default, so warning about every absent block would put
/// a notice on the admin panel for every `maven` registry in every deployment.
/// The operator expressed no belief there; there is nothing to correct.
#[test]
fn an_absent_readme_block_never_warns_even_on_an_unsupported_type() {
    let cfg = parse_config(
        r#"
        [[registries]]
        type = "maven"
        name = "reg""#,
    );
    assert!(!warning_codes(&cfg).iter().any(|c| c.starts_with("readme")));
}

/// `enabled = false` is a decision, not a mistake — nothing to warn about.
#[test]
fn a_disabled_readme_block_warns_about_nothing() {
    let cfg = config_with_readme("maven", "", "        enabled = false");
    assert!(!warning_codes(&cfg).iter().any(|c| c.starts_with("readme")));
}

/// `from_archive` on a metadata-borne-only kind is accepted and does nothing.
/// Distinct from the unsupported-type warning: READMEs *are* stored here.
#[test]
fn from_archive_on_a_metadata_borne_type_warns_that_it_is_inert() {
    let cfg = config_with_readme("jetbrains-marketplace", "", "        from_archive = true");
    let w = cfg
        .warnings()
        .into_iter()
        .find(|w| w.code == warnings::README_FROM_ARCHIVE_INERT)
        .expect("warning emitted");
    assert!(w.message.contains("stored either way"), "{}", w.message);
}

/// npm reads both, so nothing about `from_archive` is inert there.
#[test]
fn from_archive_on_a_type_that_reads_the_archive_is_quiet() {
    let cfg = config_with_readme("npm", "", "        from_archive = true");
    assert!(!warning_codes(&cfg).iter().any(|c| c.starts_with("readme")));
}

/// `firewall_only` streams without buffering, so nothing is ever cached to
/// extract from. On an archive-only kind that means no README at all, and the
/// warning says which of the two it is.
#[test]
fn from_archive_on_a_firewall_only_registry_warns_that_nothing_is_cached() {
    let cfg = config_with_readme(
        "cargo",
        "        firewall_only = true\n",
        "        from_archive = true",
    );
    let w = cfg
        .warnings()
        .into_iter()
        .find(|w| w.code == warnings::README_FROM_ARCHIVE_FIREWALL_ONLY)
        .expect("warning emitted");
    assert!(
        w.message.contains("no README will ever be stored"),
        "{}",
        w.message
    );

    // npm still has its metadata-borne half, and the message says so rather
    // than telling the operator the feature is dead.
    let npm = config_with_readme(
        "npm",
        "        firewall_only = true\n",
        "        from_archive = true",
    );
    let w = npm
        .warnings()
        .into_iter()
        .find(|w| w.code == warnings::README_FROM_ARCHIVE_FIREWALL_ONLY)
        .expect("warning emitted");
    assert!(w.message.contains("still work"), "{}", w.message);
}

// ── The console's discovery read (RFC 0007 §4.5) ──────────────────────────────

fn config_with_upstream_detail(registry_type: &str, extra: &str, block: &str) -> AppConfig {
    parse_config(&format!(
        r#"
        [[registries]]
        type = "{registry_type}"
        name = "reg"
        upstreams = ["https://example.invalid"]
{extra}
        [registries.upstream_detail]
{block}"#
    ))
}

/// Attempts the fetch and discards every result: the egress happens and nothing
/// is shown.
#[test]
fn max_versions_zero_while_enabled_refuses_to_start() {
    let cfg = config_with_upstream_detail("npm", "", "        max_versions = 0");
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("max_versions"), "{err}");
    assert!(err.contains("nothing is shown"), "{err}");
}

#[test]
fn max_versions_zero_with_the_read_disabled_is_not_an_error() {
    let cfg = config_with_upstream_detail(
        "npm",
        "",
        "        enabled = false\n        max_versions = 0",
    );
    cfg.validate()
        .expect("a disabled read fetches nothing anyway");
}

#[test]
fn max_versions_above_the_ceiling_refuses_to_start() {
    let cfg = config_with_upstream_detail("npm", "", "        max_versions = 10000");
    assert!(cfg.validate().unwrap_err().to_string().contains("ceiling"));
}

/// There is no upstream to ask, and the page is already complete from local
/// rows.
#[test]
fn the_discovery_read_on_a_local_registry_warns_that_it_is_inert() {
    let cfg = config_with_upstream_detail(
        "npm",
        "        mode = \"local\"\n",
        "        enabled = true",
    );
    let w = cfg
        .warnings()
        .into_iter()
        .find(|w| w.code == warnings::UPSTREAM_DETAIL_LOCAL_MODE)
        .expect("warning emitted");
    assert!(w.message.contains("no upstream to ask"), "{}", w.message);
    assert_eq!(w.path, "registries[0].upstream_detail");
}

/// A path-addressed kind has no package identity to ask about, and the warning
/// quotes the reason `upstream_detail()` carries — so the code and the notice
/// cannot disagree.
#[test]
fn the_discovery_read_on_an_unaskable_kind_warns_with_its_reason() {
    let cfg = parse_config(
        r#"
        [[registries]]
        type = "generic"
        name = "reg"
        upstreams = ["https://example.invalid"]
        path_allow = ["**"]

        [registries.upstream_detail]
        enabled = true"#,
    );
    let w = cfg
        .warnings()
        .into_iter()
        .find(|w| w.code == warnings::UPSTREAM_DETAIL_UNSUPPORTED_KIND)
        .expect("warning emitted");
    assert!(w.message.contains("path-addressed"), "{}", w.message);
}

/// The read is on by default, so warning about the implicit default would put a
/// notice on the admin panel for every `local`-mode registry in every
/// deployment. The operator expressed no belief there.
#[test]
fn an_absent_upstream_detail_block_never_warns() {
    let cfg = parse_config(
        r#"
        [[registries]]
        type = "npm"
        name = "reg"
        mode = "local""#,
    );
    assert!(!warning_codes(&cfg)
        .iter()
        .any(|c| c.starts_with("upstream-detail")));
}

#[test]
fn a_disabled_discovery_read_warns_about_nothing() {
    let cfg = config_with_upstream_detail(
        "npm",
        "        mode = \"local\"\n",
        "        enabled = false",
    );
    assert!(!warning_codes(&cfg)
        .iter()
        .any(|c| c.starts_with("upstream-detail")));
}

/// A proxy-mode registry of an askable kind is the case the feature was written
/// for, and it must stay quiet: a warning that fires on correct config teaches
/// operators to ignore the channel.
#[test]
fn a_correctly_configured_discovery_read_is_quiet() {
    let cfg = config_with_upstream_detail("npm", "", "        enabled = true");
    assert!(!warning_codes(&cfg)
        .iter()
        .any(|c| c.starts_with("upstream-detail")));
}

/// `remote_images = "proxy"` renders images now, so it is accepted in silence.
///
/// It used to raise `readme.image-proxy-unimplemented`, because the endpoint it
/// rewrote to did not exist. The endpoint exists (RFC 0007-bis §4.2), and a
/// warning that outlives its subject is worse than no warning — it teaches
/// operators the channel is noise.
#[test]
fn remote_images_proxy_is_accepted_without_a_warning() {
    for value in ["proxy", "strip"] {
        let cfg = config_with_readme("npm", "", &format!("        remote_images = \"{value}\""));
        cfg.validate().expect("accepted");
        assert!(
            !warning_codes(&cfg)
                .iter()
                .any(|c| c.contains("image-proxy")),
            "{value} warned about an unimplemented endpoint"
        );
    }
}

/// The image cap gets the same two guards `max_bytes` has, for the same
/// reasons: a zero serves nothing while claiming to render, and a ceiling makes
/// "held in memory while its type is checked" a bound rather than a hope
/// (RFC 0007-bis §4.5).
#[test]
fn the_image_cap_refuses_zero_under_proxy_and_refuses_an_unbounded_ceiling() {
    let zero = config_with_readme(
        "npm",
        "",
        "        remote_images = \"proxy\"\n        image_max_bytes = 0",
    );
    let err = zero.validate().expect_err("must be refused").to_string();
    assert!(err.contains("image_max_bytes"), "{err}");
    assert!(err.contains("serves no image"), "{err}");

    // Zero is fine under `strip`, where nothing is ever fetched.
    let stripped = config_with_readme(
        "npm",
        "",
        "        remote_images = \"strip\"\n        image_max_bytes = 0",
    );
    stripped.validate().expect("inert under strip");

    let huge = config_with_readme("npm", "", "        image_max_bytes = 33554432");
    let err = huge.validate().expect_err("must be refused").to_string();
    assert!(err.contains("ceiling"), "{err}");
}

// ── Prose search ──────────────────────────────────────────────────────────────

/// Off by default, and it stays off unless an operator writes it down. Unlike
/// README *capture*, which defaults on because it costs one already-parsed
/// field, this builds an index over prose (RFC 0007-bis §4.1).
#[test]
fn prose_search_is_off_unless_asked_for() {
    let cfg = parse_config("");
    assert!(!cfg.search.readmes);
    assert_eq!(cfg.search.text_config, "english");
    cfg.validate().expect("the default config is valid");
}

/// `english`, against the recommendation this RFC was drafted with. The draft
/// said stemming mangles identifiers; it does, symmetrically, and `simple` fails
/// `retry` against a README that says `retrying` (RFC 0007-bis §13.3).
#[test]
fn the_text_configuration_defaults_to_english_and_is_settable() {
    let cfg = parse_config(
        r#"
        [search]
        readmes = true
        text_config = "french""#,
    );
    cfg.validate().expect("accepted");
    assert!(cfg.search.readmes);
    assert_eq!(cfg.search.text_config, "french");
}

#[test]
fn an_empty_text_configuration_is_refused() {
    let cfg = parse_config(
        r#"
        [search]
        text_config = """#,
    );
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("text_config"), "{err}");
}

/// The index is a Postgres generated column with a GIN index. There is nowhere
/// else to put it, and failing at startup beats a search that quietly matches
/// nothing (RFC 0007-bis §4.5).
#[test]
fn prose_search_without_postgres_refuses_to_start() {
    let raw = r#"
        [database]
        type = "sqlite"
        url = "sqlite://test.db"

        [storage]
        type = "filesystem"
        path = "/tmp/batlehub-test"

        [server]
        host = "127.0.0.1"
        port = 8080

        [search]
        readmes = true
        "#;
    let cfg: AppConfig = toml::from_str(raw).expect("parses");
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("Postgres"), "{err}");

    // Both spellings of the one backend that does work are accepted.
    for spelling in ["postgres", "postgresql", "PostgreSQL"] {
        let raw = raw.replace(r#"type = "sqlite""#, &format!(r#"type = "{spelling}""#));
        let cfg: AppConfig = toml::from_str(&raw).expect("parses");
        cfg.validate().unwrap_or_else(|e| panic!("{spelling}: {e}"));
    }
}

/// Accepted, and said out loud: the index will exist and stay empty, because
/// nothing is ever stored to put in it.
#[test]
fn prose_search_over_registries_that_store_nothing_is_warned_about() {
    let cfg = parse_config(
        r#"
        [search]
        readmes = true

        [[registries]]
        type = "npm"
        name = "reg"
        upstreams = ["https://example.invalid"]
        [registries.readme]
        enabled = false"#,
    );
    cfg.validate().expect("accepted");
    let w = cfg
        .warnings()
        .into_iter()
        .find(|w| w.code == warnings::SEARCH_READMES_NOTHING_STORED)
        .expect("warning emitted");
    assert!(w.message.contains("stay empty"), "{}", w.message);

    // One registry that does capture is enough to make the setting useful, so
    // nothing is said.
    let quiet = parse_config(
        r#"
        [search]
        readmes = true

        [[registries]]
        type = "npm"
        name = "reg"
        upstreams = ["https://example.invalid"]
        [registries.readme]
        enabled = false

        [[registries]]
        type = "cargo"
        name = "reg2"
        upstreams = ["https://example.invalid"]"#,
    );
    assert!(!warning_codes(&quiet)
        .iter()
        .any(|c| c == warnings::SEARCH_READMES_NOTHING_STORED));
}

// ── `[limits].versions_per_page` ──────────────────────────────────────────────

/// The key answers one question — how much of a version list this server is
/// willing to build for one request — so it is both the default for a caller
/// that asks for nothing and the ceiling on what one may ask for. Absent means
/// the same number either way, which is what this pins: a `[limits]` block that
/// mentions something else must not read as zero.
#[test]
fn versions_per_page_defaults_to_a_hundred_with_or_without_a_limits_block() {
    let none = parse_config("");
    assert_eq!(none.limits.versions_per_page, DEFAULT_VERSIONS_PER_PAGE);

    let other_key = parse_config(
        r#"
        [limits]
        max_artifact_size_bytes = 1024"#,
    );
    assert_eq!(
        other_key.limits.versions_per_page,
        DEFAULT_VERSIONS_PER_PAGE
    );
}

/// Every caller would get an empty version list, and the failure would land on a
/// package page rather than at startup.
#[test]
fn versions_per_page_zero_refuses_to_start() {
    let cfg = parse_config(
        r#"
        [limits]
        versions_per_page = 0"#,
    );
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("versions_per_page"), "{err}");
    assert!(err.contains("empty version list"), "{err}");
}

/// Every row costs a vulnerability read and a licence read before it is
/// serialised — the same argument `upstream_detail.max_versions` makes one level
/// up.
#[test]
fn versions_per_page_above_the_ceiling_refuses_to_start() {
    let cfg = parse_config(
        r#"
        [limits]
        versions_per_page = 5000"#,
    );
    assert!(cfg.validate().unwrap_err().to_string().contains("ceiling"));
}

#[test]
fn versions_per_page_within_the_ceiling_loads() {
    let cfg = parse_config(
        r#"
        [limits]
        versions_per_page = 250"#,
    );
    cfg.validate().expect("250 is under the ceiling");
    assert_eq!(cfg.limits.versions_per_page, 250);
}

// ── `[limits].packages_per_page` ──────────────────────────────────────────────

/// The catalog's half of the same idea, with its own number because a screenful
/// of names and a query's worth of enriched version rows are not the same
/// question.
#[test]
fn packages_per_page_defaults_to_twenty() {
    assert_eq!(
        parse_config("").limits.packages_per_page,
        DEFAULT_PACKAGES_PER_PAGE
    );
    let other_key = parse_config(
        r#"
        [limits]
        versions_per_page = 50"#,
    );
    assert_eq!(
        other_key.limits.packages_per_page,
        DEFAULT_PACKAGES_PER_PAGE
    );
}

#[test]
fn packages_per_page_zero_refuses_to_start() {
    let cfg = parse_config(
        r#"
        [limits]
        packages_per_page = 0"#,
    );
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("packages_per_page"), "{err}");
    assert!(err.contains("empty package catalog"), "{err}");
}

#[test]
fn packages_per_page_above_the_ceiling_refuses_to_start() {
    let cfg = parse_config(
        r#"
        [limits]
        packages_per_page = 5000"#,
    );
    assert!(cfg.validate().unwrap_err().to_string().contains("ceiling"));
}

/// The two keys are independent: setting one must not move the other.
#[test]
fn the_two_page_sizes_do_not_move_each_other() {
    let cfg = parse_config(
        r#"
        [limits]
        packages_per_page = 40"#,
    );
    cfg.validate().expect("40 is a fine catalog page");
    assert_eq!(cfg.limits.packages_per_page, 40);
    assert_eq!(cfg.limits.versions_per_page, DEFAULT_VERSIONS_PER_PAGE);
}

// ── Auth: transport and identity ──────────────────────────────────────────────

/// The Kubernetes API server is held to the same rule as an OIDC issuer, and for
/// a stronger reason: the TokenReview carries this server's own service account
/// token, and whoever answers it decides the caller's role.
#[test]
fn a_plain_http_kubernetes_api_server_refuses_to_start() {
    let cfg = parse_config(
        r#"
        [[auth]]
        type = "kubernetes"
        name = "k8s"
        api_server = "http://kube.internal:6443""#,
    );
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("api_server"), "{err}");
    assert!(err.contains("must use https"), "{err}");
    assert!(
        err.contains("service account token"),
        "the message should say what is at stake; {err}"
    );
}

/// The same userinfo trick `is_secure_issuer_url` refuses for an issuer: the
/// authority ends at the last `@`, so this dials `evil.example` in cleartext.
#[test]
fn a_kubernetes_api_server_hiding_behind_userinfo_refuses_to_start() {
    let cfg = parse_config(
        r#"
        [[auth]]
        type = "kubernetes"
        name = "k8s"
        api_server = "http://localhost:6443@evil.example""#,
    );
    assert!(cfg
        .validate()
        .unwrap_err()
        .to_string()
        .contains("must use https"));
}

/// Loopback over plain HTTP stays allowed — that is how the test suites and a
/// developer's local cluster run — and an absent `api_server` has nothing to
/// check: it means the in-cluster default, which is https by construction.
#[test]
fn a_loopback_or_absent_kubernetes_api_server_starts() {
    parse_config(
        r#"
        [[auth]]
        type = "kubernetes"
        name = "k8s"
        api_server = "http://127.0.0.1:6443""#,
    )
    .validate()
    .expect("loopback is exempt");

    parse_config(
        r#"
        [[auth]]
        type = "kubernetes"
        name = "k8s""#,
    )
    .validate()
    .expect("an absent api_server means the in-cluster https default");
}

/// A blank `audience` is a configuration error, not an unreachable provider.
///
/// `ActionsOidcAuthProvider::new` refuses it, but that error is routed through
/// `provider_unavailable(…, cfg.required, …)` and `required` defaults to `false`
/// for this kind — so the server used to log one warning and come up *without*
/// the provider, silently downgrading every CI caller to anonymous.
#[test]
fn a_blank_actions_oidc_audience_is_refused_at_load() {
    for audience in ["\"\"", "\"   \""] {
        let cfg = parse_config(&format!(
            r#"
        [[auth]]
        type = "actions-oidc"
        name = "gha"
        issuer_url = "https://token.actions.githubusercontent.com"
        audience = {audience}"#
        ));
        let err = cfg
            .validate()
            .expect_err("a blank audience must not start")
            .to_string();
        assert!(err.contains("audience"), "{err}");
        assert!(err.contains("gha"), "{err}");
    }
}

/// And a real one still starts — the check is about blankness, not about the
/// provider kind being unwelcome.
#[test]
fn an_actions_oidc_provider_with_an_audience_starts() {
    parse_config(
        r#"
        [[auth]]
        type = "actions-oidc"
        name = "gha"
        issuer_url = "https://token.actions.githubusercontent.com"
        audience = "https://batlehub.example.com""#,
    )
    .validate()
    .expect("a named audience is the supported configuration");
}

/// Two providers of different kinds sharing one name are one provider as far as
/// `oidc_session_owner` and the group prefix are concerned — which is how a
/// service account comes to mint personal access tokens as an OIDC user.
#[test]
fn two_auth_providers_may_not_share_a_name() {
    let cfg = parse_config(
        r#"
        [[auth]]
        type = "oidc"
        name = "corp"
        issuer_url = "https://idp.example.com"
        client_id = "batlehub"

        [[auth]]
        type = "kubernetes"
        name = "corp""#,
    );
    let err = cfg.validate().unwrap_err().to_string();
    assert!(err.contains("\"corp\""), "{err}");
    assert!(err.contains("oidc"), "{err}");
    assert!(err.contains("kubernetes"), "{err}");
}

/// Including two of the same kind: the collision is about the name, not about
/// the kinds differing.
#[test]
fn two_oidc_providers_may_not_share_a_name_either() {
    let cfg = parse_config(
        r#"
        [[auth]]
        type = "oidc"
        name = "corp"
        issuer_url = "https://idp.example.com"
        client_id = "batlehub"

        [[auth]]
        type = "oidc"
        name = "corp"
        issuer_url = "https://other.example.com"
        client_id = "batlehub""#,
    );
    assert!(cfg.validate().is_err());
}

/// Distinct names are the normal case and must still start, static tokens
/// included — that provider has no name of its own to collide with.
#[test]
fn distinct_auth_names_start() {
    parse_config(
        r#"
        [[auth]]
        type = "token"

        [[auth]]
        type = "oidc"
        name = "corp"
        issuer_url = "https://idp.example.com"
        client_id = "batlehub"

        [[auth]]
        type = "kubernetes"
        name = "k8s-prod""#,
    )
    .validate()
    .expect("distinct names are fine");
}
