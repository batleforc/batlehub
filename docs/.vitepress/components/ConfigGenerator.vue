<script setup lang="ts">
import { ref, computed, watch, onMounted } from "vue";

// ── Types ───────────────────────────────────────────────────────────────────

type RegistryMode = "proxy" | "local" | "hybrid";
type RegistryType =
  | "npm"
  | "cargo"
  | "openvsx"
  | "vscode-marketplace"
  | "goproxy"
  | "github"
  | "forgejo"
  | "gitlab"
  | "maven"
  | "terraform"
  | "rubygems"
  | "composer"
  | "pypi"
  | "conda"
  | "nuget"
  | "deb"
  | "rpm"
  | "pacman"
  | "jetbrains"
  | "jetbrains-marketplace"
  | "generic";
type AuthRole = "admin" | "user" | "anonymous";
type StorageBackendType = "filesystem" | "s3";
type StorageMode = "single" | "multi";
type AuthType = "token" | "oidc" | "kubernetes" | "actions-oidc";
type UpstreamAuthType = "" | "bearer" | "basic" | "header";
type Enforcement = "block" | "warn";
type MatchMode = "all" | "any";
type ConditionMatchType = "auto" | "glob" | "regex";

interface StorageBackend {
  id: number;
  name: string;
  type: StorageBackendType;
  path: string;
  bucket: string;
  region: string;
  endpoint_url: string;
  force_path_style: boolean;
  prefix: string;
}

interface Token {
  id: number;
  value: string;
  role: AuthRole;
  user_id: string;
}

interface Condition {
  id: number;
  claim: string;
  pattern: string;
  match_type: ConditionMatchType;
}

interface ActionsRule {
  id: number;
  group: string;
  group_template: string;
  role: string; // "" = omitted (group-only rule)
  match_mode: MatchMode;
  conditions: Condition[];
}

/// One `claim value -> proxy role` entry of an OIDC/Kubernetes `role_mappings`
/// table. Without at least one of these every federated identity lands on
/// `anonymous`, so a generated SSO config would have no administrator at all.
interface RoleMapping {
  id: number;
  claim: string;
  role: string;
}

/// One entry of `[registries.rbac.groups]` — a group name (`"oidc:team-a"`, or
/// `"*:team-a"` to match the group across every provider) and its permissions.
interface RbacGroup {
  id: number;
  name: string;
  perms: string;
}

/// One `[[registries.rate_limit.groups]]` override: a group whose members get a
/// different budget from the registry-wide one.
interface RateLimitGroup {
  id: number;
  name: string;
  requests_per_window: number;
  window_secs: number;
  enforcement: "" | Enforcement;
}

type NotifChannelType = "webhook" | "slack" | "teams" | "email";

interface NotifChannel {
  id: number;
  type: NotifChannelType;
  name: string;
  url: string;
  secret: string;
  timeout_secs: number;
  // email only
  smtp_host: string;
  smtp_port: number;
  smtp_user: string;
  smtp_password: string;
  from: string;
  to: string;
  tls: boolean;
}

interface InboundHook {
  id: number;
  name: string;
  secret: string;
}

interface AuthProvider {
  id: number;
  type: AuthType;
  // token
  tokens: Token[];
  // oidc
  oidc_name: string;
  oidc_issuer: string;
  oidc_client_id: string;
  oidc_client_secret: string;
  oidc_redirect_uri: string;
  oidc_frontend_url: string;
  oidc_user_id_claim: string;
  oidc_role_claim: string;
  oidc_scopes: string;
  oidc_role_mappings: RoleMapping[];
  // kubernetes
  k8s_name: string;
  k8s_api_server: string;
  k8s_ca_cert_path: string;
  k8s_token_path: string;
  k8s_audiences: string;
  k8s_role_mappings: RoleMapping[];
  // actions-oidc
  actions_name: string;
  actions_issuer: string;
  actions_user_id_claim: string;
  actions_rules: ActionsRule[];
}

interface Registry {
  id: number;
  name: string;
  type: RegistryType;
  mode: RegistryMode;
  upstreams: string;
  storage_backend: string;
  rbac_anonymous: string;
  rbac_user: string;
  rbac_admin: string;
  rbac_groups: RbacGroup[];
  rbac_explore_anonymous: boolean;
  rbac_explore_user: boolean;
  rbac_explore_admin: boolean;
  showAdvanced: boolean;
  // routing / addressing
  hosts: string;
  path_routing: boolean;
  path_allow: string;
  index_url: string;
  search_url: string;
  search_url_disabled: boolean;
  vuln_db_url: string;
  vuln_db_url_disabled: boolean;
  // upstream auth
  upstream_auth_type: UpstreamAuthType;
  upstream_auth_token: string;
  upstream_auth_username: string;
  upstream_auth_password: string;
  upstream_auth_header_name: string;
  upstream_auth_header_value: string;
  // tls
  tls_ca_cert_path: string;
  // per-registry egress proxy
  proxy_enabled: boolean;
  proxy_url: string;
  proxy_username: string;
  proxy_password: string;
  proxy_no_proxy: string;
  // firewall
  firewall_only: boolean;
  // cache policy
  cache_metadata_ttl: number;
  cache_artifact_ttl: string;
  cache_idle_days: string;
  cache_max_size_bytes: string;
  cache_keep_latest_n: string;
  cache_serve_stale: boolean;
  cache_warm_packages: string;
  cache_warm_paths: string;
  cache_warm_latest_n: number;
  cache_warm_concurrency: number;
  // rate limit
  rate_limit_enabled: boolean;
  rate_limit_rps: number;
  rate_limit_window: number;
  rate_limit_enforcement: Enforcement;
  rate_limit_groups: RateLimitGroup[];
  // quota (local/hybrid)
  quota_enabled: boolean;
  quota_max_bytes: string;
  quota_max_packages: string;
  quota_warn_threshold_pct: number;
  quota_enforcement: Enforcement;
  // beta channel (local/hybrid)
  beta_channel_enabled: boolean;
  // versioning (local/hybrid)
  versioning_enabled: boolean;
  versioning_enforce_semver: boolean;
  versioning_allow_prerelease: boolean;
  versioning_pattern: string;
  // signing (local/hybrid)
  signing_enabled: boolean;
  signing_required: boolean;
  signing_allowed_types: string;
  signing_verify_on_download: boolean;
  signing_trusted_keys: string;
  // repo metadata signing (deb/rpm)
  repo_signing_enabled: boolean;
  repo_signing_seed_hex: string;
  repo_signing_user_id: string;
  repo_signing_created: string;
  // sbom
  sbom_enabled: boolean;
  sbom_formats: string;
  sbom_required: boolean;
  sbom_fetch_upstream: boolean;
  // integrity
  integrity_customised: boolean;
  integrity_enabled: boolean;
  integrity_block_on_mismatch: boolean;
  integrity_require_metadata: boolean;
  integrity_bypass_roles: string;
  integrity_verify_on_serve: boolean;
  // rules
  rule_age_gate_enabled: boolean;
  rule_age_gate_min_age: number;
  rule_age_gate_bypass_roles: string;
  rule_age_gate_deny_missing_timestamp: boolean;
  rule_deny_latest_enabled: boolean;
  rule_deny_latest_bypass_roles: string;
  rule_signed_release_enabled: boolean;
  rule_signed_release_bypass_roles: string;
  rule_signed_release_deny_missing: boolean;
  rule_license_gate_enabled: boolean;
  rule_license_gate_allow: string;
  rule_license_gate_deny: string;
  rule_license_gate_allow_unknown: boolean;
  rule_license_gate_block: boolean;
  rule_license_gate_bypass_roles: string;
  rule_version_gate_enabled: boolean;
  rule_version_gate_allow: string;
  rule_version_gate_block: string;
  rule_version_gate_bypass_roles: string;
  rule_cve_gate_enabled: boolean;
  rule_cve_gate_min_severity: string;
  rule_cve_gate_block: boolean;
  rule_cve_gate_bypass_roles: string;
  rule_trusted_publisher_enabled: boolean;
  rule_trusted_publisher_allow: string;
  rule_trusted_publisher_bypass_roles: string;
  // feature flags
  feature_flags_socket_badge: boolean;
}

// ── State ───────────────────────────────────────────────────────────────────

// Mirrors `CURRENT_CONFIG_VERSION` in crates/config/src/schema/mod.rs. Bump both
// together: a config declaring a version the binary does not know is rejected.
const CONFIG_VERSION = 1;

const server = ref({
  host: "0.0.0.0",
  port: 8080,
  static_dir: "",
  cli_binary_path: "",
  cors_allowed_origins: "",
  // `[server].trusted_proxies` supersedes the deprecated `[ip_blocking]` key of
  // the same name; setting it here is what stops the server logging the
  // deprecation warning at startup. Unlike the old key an empty *list* is a
  // policy ("trust nobody"), so the checkbox decides whether the key is written
  // at all and the text decides its contents.
  trusted_proxies_set: false,
  trusted_proxies: "",
});
const database = ref({
  url: "",
  max_connections: 10,
  min_connections: 1,
  acquire_timeout_secs: 30,
});

const metaCache = ref({ type: "memory", url: "" });

const limits = ref({ max_artifact_size_bytes: "" });

const ipBlocking = ref({
  enabled: false,
  violation_threshold: 10,
  violation_window_secs: 300,
  ban_duration_secs: 3600,
  trigger_on_status: "429, 401",
});

const vulnerabilityScan = ref({
  enabled: false,
  interval_secs: 86400,
  osv_api_url: "",
  batch_size: 100,
});

const stats = ref({
  history_enabled: true,
  // 30, matching `default_history_retention_days` — showing any other number
  // here would promise a retention the server does not apply, since the key is
  // only written when it differs from the default.
  history_retention_days: 30,
  metrics_enabled: true,
});

const subdomainRouting = ref({
  enabled: false,
  base_domain: "",
  scheme: "https",
});

// Global egress proxy — inherited by every registry that does not set its own.
const upstreamProxy = ref({
  enabled: false,
  url: "",
  username: "",
  password: "",
  no_proxy: "",
});

let channelSeq = 0;
let inboundSeq = 0;
const notifications = ref({
  enabled: false,
  channels: [] as NotifChannel[],
  inbound: [] as InboundHook[],
});

// Storage
const storageMode = ref<StorageMode>("single");
const singleStorage = ref<{
  type: StorageBackendType;
  path: string;
  bucket: string;
  region: string;
  endpoint_url: string;
  force_path_style: boolean;
  prefix: string;
}>({
  type: "filesystem",
  path: "./cache",
  bucket: "",
  region: "us-east-1",
  endpoint_url: "",
  force_path_style: false,
  prefix: "",
});
let backendSeq = 0;
const storageDefault = ref("primary");
const storageBackends = ref<StorageBackend[]>([
  {
    id: backendSeq++,
    name: "primary",
    type: "filesystem",
    path: "./cache",
    bucket: "",
    region: "us-east-1",
    endpoint_url: "",
    force_path_style: false,
    prefix: "",
  },
]);

// OTel
const otel = ref({
  enabled: false,
  endpoint: "http://localhost:4317",
  service_name: "batlehub",
});

// Auth providers
let authSeq = 0;
let tokenSeq = 0;
let ruleSeq = 0;
let condSeq = 0;
let mappingSeq = 0;

function blankAuthProvider(): AuthProvider {
  return {
    id: authSeq++,
    type: "token",
    tokens: [],
    oidc_name: "",
    oidc_issuer: "",
    oidc_client_id: "",
    oidc_client_secret: "",
    oidc_redirect_uri: "",
    oidc_frontend_url: "",
    oidc_user_id_claim: "sub",
    oidc_role_claim: "role",
    oidc_scopes: "",
    oidc_role_mappings: [{ id: mappingSeq++, claim: "batlehub-admins", role: "admin" }],
    k8s_name: "",
    k8s_api_server: "",
    k8s_ca_cert_path: "",
    k8s_token_path: "",
    k8s_audiences: "batlehub",
    k8s_role_mappings: [
      { id: mappingSeq++, claim: "system:serviceaccount:ci:builder", role: "user" },
    ],
    actions_name: "",
    actions_issuer: "",
    actions_user_id_claim: "sub",
    actions_rules: [],
  };
}

const authProviders = ref<AuthProvider[]>([
  {
    ...blankAuthProvider(),
    tokens: [{ id: tokenSeq++, value: "", role: "admin", user_id: "admin" }],
  },
]);

// ── Argon2id token hashing ───────────────────────────────────────────────────

type ArgFn = (params: {
  password: string;
  salt: Uint8Array;
  parallelism: number;
  iterations: number;
  memorySize: number;
  hashLength: number;
  outputType: "encoded";
}) => Promise<string>;

let _argon2id: ArgFn | null = null;
const tokenHashes = ref<Record<number, string | null>>({});
let _hashTimer: ReturnType<typeof setTimeout> | null = null;

/**
 * Generation counter for `runHashComputation`.
 *
 * Argon2id at `memorySize: 65536, iterations: 3` takes well over a second in a
 * browser, which is far longer than the 350 ms debounce — so runs overlap, and
 * `scheduleHashing` can only cancel a run that has not *started*. Without this
 * guard the slower, older run wrote last: type `secret-a`, pause, type `-b`,
 * pause, and run B finishes first with `hash("secret-a-b")` before run A lands
 * and overwrites it with `hash("secret-a")`.
 *
 * The panel then shows a green `$argon2id$…` and emits it into
 * `[[auth.tokens]] value = …` — a correct-looking config holding the hash of a
 * token the operator never had, discovered only as a failed login. A stale
 * result must be dropped, not written.
 */
let _hashRun = 0;

async function runHashComputation() {
  const mine = ++_hashRun;

  const next: Record<number, string | null> = {};
  for (const auth of authProviders.value) {
    if (auth.type !== "token") continue;
    for (const tok of auth.tokens) {
      next[tok.id] = tok.value.trim() ? null : "";
    }
  }
  tokenHashes.value = next;

  for (const auth of authProviders.value) {
    if (auth.type !== "token") continue;
    for (const tok of auth.tokens) {
      const raw = tok.value.trim();
      if (!raw) continue;
      try {
        let result: string;
        if (_argon2id) {
          const salt = new Uint8Array(16);
          crypto.getRandomValues(salt);
          result = await _argon2id({
            password: raw,
            salt,
            parallelism: 4,
            iterations: 3,
            memorySize: 65536,
            hashLength: 32,
            outputType: "encoded",
          });
        } else {
          result = raw;
        }
        if (mine !== _hashRun) return; // superseded — this hash is of stale input
        tokenHashes.value = { ...tokenHashes.value, [tok.id]: result };
      } catch {
        if (mine !== _hashRun) return;
        tokenHashes.value = { ...tokenHashes.value, [tok.id]: raw };
      }
    }
  }
}

function scheduleHashing() {
  if (_hashTimer) clearTimeout(_hashTimer);
  // Clearing the timer only cancels a run that has not started; one already
  // in flight is stranded by the generation counter above.
  _hashTimer = setTimeout(() => void runHashComputation(), 350);
}

onMounted(async () => {
  try {
    const mod = await import("hash-wasm");
    _argon2id = mod.argon2id as unknown as ArgFn;
  } catch {
    // hash-wasm failed to load — plain-text fallback
  }
  await runHashComputation();
});

watch(
  () =>
    authProviders.value.flatMap((a) =>
      a.type === "token" ? a.tokens.map((t) => `${t.id}:${t.value}`) : [],
    ),
  scheduleHashing,
);

// ── Registries
let registrySeq = 0;

const defaultUpstream: Record<RegistryType, string> = {
  npm: "https://registry.npmjs.org",
  cargo: "https://index.crates.io",
  openvsx: "https://open-vsx.org",
  "vscode-marketplace": "https://marketplace.visualstudio.com",
  goproxy: "https://proxy.golang.org",
  github: "https://api.github.com",
  forgejo: "https://codeberg.org",
  gitlab: "https://gitlab.com",
  maven: "https://repo1.maven.org/maven2",
  terraform: "https://registry.terraform.io",
  rubygems: "https://rubygems.org",
  composer: "https://repo.packagist.org",
  pypi: "https://pypi.org",
  conda: "https://conda.anaconda.org",
  nuget: "https://api.nuget.org",
  // Deb has a canonical Debian mirror; RPM and generic have no universal default
  // upstream, so they are left blank for the user to fill in (e.g. a
  // Fedora/openSUSE mirror). The backend rejects those two at startup when the
  // list is empty rather than falling back to an unreachable placeholder.
  deb: "https://deb.debian.org",
  rpm: "",
  pacman: "https://geo.mirror.pkgbuild.com",
  jetbrains: "https://download.jetbrains.com",
  "jetbrains-marketplace": "https://plugins.jetbrains.com",
  generic: "",
};

// Kinds addressed purely by upstream file path — the only ones for which
// `path_allow` and `cache.warm_paths` mean anything (the backend rejects
// `path_allow` on any other kind). Mirrors `RegistryKind::is_path_addressed`.
const PATH_ADDRESSED_TYPES = new Set<RegistryType>([
  "deb",
  "rpm",
  "pacman",
  "jetbrains",
  "generic",
]);
const isPathAddressed = (reg: Registry) => PATH_ADDRESSED_TYPES.has(reg.type);

// `deb`/`rpm` registries are the ones that publish signed repository metadata.
const REPO_SIGNING_TYPES = new Set<RegistryType>(["deb", "rpm"]);

function defaultRegistry(type: RegistryType = "npm"): Registry {
  return {
    id: registrySeq++,
    name: type,
    type,
    mode: "proxy",
    upstreams: defaultUpstream[type],
    storage_backend: "",
    rbac_anonymous: "releases:read, source:read",
    rbac_user: "releases:read, source:read",
    rbac_admin: "*",
    rbac_groups: [],
    rbac_explore_anonymous: true,
    rbac_explore_user: true,
    rbac_explore_admin: true,
    showAdvanced: false,
    hosts: "",
    path_routing: true,
    // `generic` is the one kind the backend refuses to start without an
    // allowlist, so it is seeded with the explicit mirror-everything opt-out
    // rather than a blank field that fails validation.
    path_allow: type === "generic" ? "**" : "",
    index_url: "",
    search_url: "",
    search_url_disabled: false,
    vuln_db_url: "",
    vuln_db_url_disabled: false,
    upstream_auth_type: "",
    upstream_auth_token: "",
    upstream_auth_username: "",
    upstream_auth_password: "",
    upstream_auth_header_name: "",
    upstream_auth_header_value: "",
    tls_ca_cert_path: "",
    proxy_enabled: false,
    proxy_url: "",
    proxy_username: "",
    proxy_password: "",
    proxy_no_proxy: "",
    firewall_only: false,
    cache_metadata_ttl: 300,
    cache_artifact_ttl: "",
    cache_idle_days: "",
    cache_max_size_bytes: "",
    cache_keep_latest_n: "",
    cache_serve_stale: true,
    cache_warm_packages: "",
    cache_warm_paths: "",
    cache_warm_latest_n: 1,
    cache_warm_concurrency: 2,
    rate_limit_enabled: false,
    rate_limit_rps: 100,
    rate_limit_window: 60,
    rate_limit_enforcement: "block",
    rate_limit_groups: [],
    quota_enabled: false,
    quota_max_bytes: "",
    quota_max_packages: "",
    quota_warn_threshold_pct: 80,
    quota_enforcement: "block",
    beta_channel_enabled: false,
    versioning_enabled: false,
    versioning_enforce_semver: false,
    versioning_allow_prerelease: true,
    versioning_pattern: "",
    signing_enabled: false,
    signing_required: false,
    signing_allowed_types: "",
    signing_verify_on_download: false,
    signing_trusted_keys: "",
    repo_signing_enabled: false,
    repo_signing_seed_hex: "",
    repo_signing_user_id: "",
    repo_signing_created: "",
    sbom_enabled: false,
    sbom_formats: "spdx, cyclonedx",
    sbom_required: false,
    sbom_fetch_upstream: true,
    integrity_customised: false,
    integrity_enabled: true,
    integrity_block_on_mismatch: true,
    integrity_require_metadata: false,
    integrity_bypass_roles: "admin",
    integrity_verify_on_serve: false,
    rule_age_gate_enabled: false,
    rule_age_gate_min_age: 3600,
    rule_age_gate_bypass_roles: "admin",
    rule_age_gate_deny_missing_timestamp: false,
    rule_deny_latest_enabled: false,
    rule_deny_latest_bypass_roles: "admin",
    rule_signed_release_enabled: false,
    rule_signed_release_bypass_roles: "admin",
    rule_signed_release_deny_missing: false,
    rule_license_gate_enabled: false,
    rule_license_gate_allow: "",
    rule_license_gate_deny: "",
    rule_license_gate_allow_unknown: true,
    rule_license_gate_block: false,
    rule_license_gate_bypass_roles: "admin",
    rule_version_gate_enabled: false,
    rule_version_gate_allow: "",
    rule_version_gate_block: "",
    rule_version_gate_bypass_roles: "admin",
    rule_cve_gate_enabled: false,
    rule_cve_gate_min_severity: "high",
    rule_cve_gate_block: false,
    rule_cve_gate_bypass_roles: "admin",
    rule_trusted_publisher_enabled: false,
    rule_trusted_publisher_allow: "",
    rule_trusted_publisher_bypass_roles: "admin",
    feature_flags_socket_badge: true,
  };
}

const registries = ref<Registry[]>([defaultRegistry("npm")]);

// ── Helpers ─────────────────────────────────────────────────────────────────

function q(s: string) {
  return `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

function permsToToml(csv: string): string {
  const perms = csv
    .split(",")
    .map((p) => p.trim())
    .filter(Boolean);
  if (!perms.length) return "[]";
  return `[${perms.map(q).join(", ")}]`;
}

function csvToList(csv: string): string[] {
  return csv
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

/// Splits a textarea's contents into one entry per line. Used where an entry may
/// legitimately contain a comma (glob patterns, semver ranges like
/// `">=1.2.0, <2.0.0"`), which rules out the comma-separated inputs.
function linesToList(text: string): string[] {
  return text
    .split("\n")
    .map((s) => s.trim())
    .filter(Boolean);
}

function tomlArray(items: string[]): string {
  return `[${items.map(q).join(", ")}]`;
}

/// Comma-separated numbers as an *unquoted* TOML array. `trigger_on_status` is
/// a `Vec<u16>` on the Rust side, so quoting the entries makes the whole config
/// fail to parse ("invalid type: string, expected u16"). Non-numeric input is
/// dropped rather than passed through, for the same reason.
function numListToToml(csv: string): string {
  const nums = csv
    .split(",")
    .map((n) => n.trim())
    .filter((n) => /^\d+$/.test(n));
  return `[${nums.join(", ")}]`;
}

function listToToml(csv: string): string {
  const items = csv
    .split(",")
    .map((p) => p.trim())
    .filter(Boolean);
  if (!items.length) return "[]";
  return `[${items.map(q).join(", ")}]`;
}

function backendFields(b: {
  type: StorageBackendType;
  path: string;
  bucket: string;
  region: string;
  endpoint_url: string;
  force_path_style: boolean;
  prefix: string;
}): string[] {
  const lines: string[] = [];
  lines.push(`type = ${q(b.type)}`);
  if (b.type === "filesystem") {
    lines.push(`path = ${q(b.path || "./cache")}`);
  } else {
    lines.push(`bucket = ${q(b.bucket)}`);
    lines.push(`region = ${q(b.region)}`);
    if (b.prefix) lines.push(`prefix = ${q(b.prefix)}`);
    if (b.endpoint_url) lines.push(`endpoint_url = ${q(b.endpoint_url)}`);
    if (b.force_path_style) lines.push(`force_path_style = true`);
  }
  return lines;
}

// ── TOML generation ─────────────────────────────────────────────────────────

const toml = computed(() => {
  const lines: string[] = [];

  // Declares which schema generation this file targets. The server refuses a
  // version newer than it understands instead of silently ignoring keys.
  lines.push(`config_version = ${CONFIG_VERSION}`);
  lines.push("");

  // [server]
  lines.push("[server]");
  lines.push(`host = ${q(server.value.host)}`);
  lines.push(`port = ${server.value.port}`);
  if (server.value.static_dir)
    lines.push(`static_dir = ${q(server.value.static_dir)}`);
  if (server.value.cli_binary_path)
    lines.push(`cli_binary_path = ${q(server.value.cli_binary_path)}`);
  if (server.value.cors_allowed_origins) {
    lines.push(`cors_allowed_origins = ${listToToml(server.value.cors_allowed_origins)}`);
  }
  if (server.value.trusted_proxies_set) {
    lines.push(`trusted_proxies = ${listToToml(server.value.trusted_proxies)}`);
  }

  // [database]
  lines.push("");
  lines.push("[database]");
  lines.push(`type = "postgresql"`);
  lines.push(
    `url = ${q(database.value.url || "postgresql://batlehub:changeme@localhost:5432/batlehub")}`,
  );
  if (database.value.max_connections !== 10)
    lines.push(`max_connections = ${database.value.max_connections}`);
  if (database.value.min_connections !== 1)
    lines.push(`min_connections = ${database.value.min_connections}`);
  if (database.value.acquire_timeout_secs !== 30)
    lines.push(`acquire_timeout_secs = ${database.value.acquire_timeout_secs}`);

  // [cache]
  if (metaCache.value.type !== "memory" || metaCache.value.url) {
    lines.push("");
    lines.push("[cache]");
    lines.push(`type = ${q(metaCache.value.type)}`);
    if (metaCache.value.type === "redis" && metaCache.value.url) {
      lines.push(`url = ${q(metaCache.value.url)}`);
    }
  }

  // [limits]
  if (limits.value.max_artifact_size_bytes) {
    lines.push("");
    lines.push("[limits]");
    lines.push(
      `max_artifact_size_bytes = ${limits.value.max_artifact_size_bytes}`,
    );
  }

  // [[auth]]
  for (const auth of authProviders.value) {
    lines.push("");
    lines.push("[[auth]]");
    lines.push(`type = ${q(auth.type)}`);

    if (auth.type === "token") {
      const valid = auth.tokens.filter((t) => t.value.trim());
      for (const tok of valid) {
        lines.push("");
        lines.push("[[auth.tokens]]");
        const hash = tokenHashes.value[tok.id];
        if (hash === null) {
          lines.push(`value = "# computing Argon2id hash…"`);
        } else if (hash && hash.startsWith("$argon2")) {
          lines.push(`value = ${q(hash)}`);
        } else {
          lines.push(`# Argon2id hashing unavailable in this browser.`);
          lines.push(`# Harden this token: batlehub hash-token ${tok.value}`);
          lines.push(`value = ${q(tok.value)}`);
        }
        lines.push(`role = ${q(tok.role)}`);
        if (tok.user_id) lines.push(`user_id = ${q(tok.user_id)}`);
      }
    } else if (auth.type === "oidc") {
      if (auth.oidc_name) lines.push(`name = ${q(auth.oidc_name)}`);
      if (auth.oidc_issuer) lines.push(`issuer_url = ${q(auth.oidc_issuer)}`);
      if (auth.oidc_client_id)
        lines.push(`client_id = ${q(auth.oidc_client_id)}`);
      if (auth.oidc_client_secret)
        lines.push(`client_secret = ${q(auth.oidc_client_secret)}`);
      if (auth.oidc_redirect_uri)
        lines.push(`redirect_uri = ${q(auth.oidc_redirect_uri)}`);
      if (auth.oidc_frontend_url)
        lines.push(`frontend_url = ${q(auth.oidc_frontend_url)}`);
      if (auth.oidc_user_id_claim && auth.oidc_user_id_claim !== "sub")
        lines.push(`user_id_claim = ${q(auth.oidc_user_id_claim)}`);
      if (auth.oidc_role_claim && auth.oidc_role_claim !== "role")
        lines.push(`role_claim = ${q(auth.oidc_role_claim)}`);
      if (auth.oidc_scopes) {
        const scopes = auth.oidc_scopes.split(",").map((s) => s.trim()).filter(Boolean);
        if (scopes.length) lines.push(`scopes = [${scopes.map(q).join(", ")}]`);
      }
      // Claim values with no mapping fall back to `anonymous`, so a provider
      // with no entries here authenticates people into having no rights at all.
      const oidcMappings = auth.oidc_role_mappings.filter((m) => m.claim.trim());
      if (oidcMappings.length) {
        lines.push("");
        lines.push("[auth.role_mappings]");
        for (const m of oidcMappings) {
          lines.push(`${q(m.claim.trim())} = ${q(m.role)}`);
        }
      }
    } else if (auth.type === "kubernetes") {
      if (auth.k8s_name) lines.push(`name = ${q(auth.k8s_name)}`);
      if (auth.k8s_api_server)
        lines.push(`api_server = ${q(auth.k8s_api_server)}`);
      if (auth.k8s_ca_cert_path)
        lines.push(`ca_cert_path = ${q(auth.k8s_ca_cert_path)}`);
      if (auth.k8s_token_path)
        lines.push(`token_path = ${q(auth.k8s_token_path)}`);
      if (auth.k8s_audiences) {
        const auds = auth.k8s_audiences
          .split(",")
          .map((a) => a.trim())
          .filter(Boolean);
        lines.push(`audiences = [${auds.map(q).join(", ")}]`);
      }
      // Keys are Kubernetes usernames (`system:serviceaccount:<ns>:<name>`) or
      // group names; unmapped identities land on `anonymous`.
      const k8sMappings = auth.k8s_role_mappings.filter((m) => m.claim.trim());
      if (k8sMappings.length) {
        lines.push("");
        lines.push("[auth.role_mappings]");
        for (const m of k8sMappings) {
          lines.push(`${q(m.claim.trim())} = ${q(m.role)}`);
        }
      }
    } else if (auth.type === "actions-oidc") {
      if (auth.actions_name) lines.push(`name = ${q(auth.actions_name)}`);
      if (auth.actions_issuer) lines.push(`issuer_url = ${q(auth.actions_issuer)}`);
      if (auth.actions_user_id_claim && auth.actions_user_id_claim !== "sub")
        lines.push(`user_id_claim = ${q(auth.actions_user_id_claim)}`);
      for (const rule of auth.actions_rules) {
        lines.push("");
        lines.push("[[auth.rules]]");
        if (rule.group) lines.push(`group = ${q(rule.group)}`);
        if (rule.group_template) lines.push(`group_template = ${q(rule.group_template)}`);
        if (rule.role) lines.push(`role = ${q(rule.role)}`);
        if (rule.match_mode !== "all") lines.push(`match = ${q(rule.match_mode)}`);
        for (const cond of rule.conditions) {
          lines.push("");
          lines.push("[[auth.rules.conditions]]");
          lines.push(`claim = ${q(cond.claim)}`);
          lines.push(`pattern = ${q(cond.pattern)}`);
          if (cond.match_type !== "auto") lines.push(`match_type = ${q(cond.match_type)}`);
        }
      }
    }
  }

  // [storage]
  lines.push("");
  if (storageMode.value === "single") {
    lines.push("[storage]");
    for (const l of backendFields(singleStorage.value)) lines.push(l);
  } else {
    lines.push("[storage]");
    lines.push(`default = ${q(storageDefault.value)}`);
    for (const b of storageBackends.value) {
      if (!b.name) continue;
      lines.push("");
      lines.push("[[storage.backends]]");
      lines.push(`name = ${q(b.name)}`);
      for (const l of backendFields(b)) lines.push(l);
    }
  }

  // [[registries]]
  for (const reg of registries.value) {
    if (!reg.name) continue;
    lines.push("");
    lines.push("[[registries]]");
    lines.push(`type = ${q(reg.type)}`);
    lines.push(`name = ${q(reg.name)}`);
    if (reg.mode !== "proxy") lines.push(`mode = ${q(reg.mode)}`);
    if (reg.firewall_only) lines.push(`firewall_only = true`);
    if (reg.mode !== "local") {
      const ups = reg.upstreams
        .split("\n")
        .map((u) => u.trim())
        .filter(Boolean);
      if (ups.length) lines.push(`upstreams = [${ups.map(q).join(", ")}]`);
    }
    if (storageMode.value === "multi" && reg.storage_backend) {
      lines.push(`storage = ${q(reg.storage_backend)}`);
    }
    if (reg.type === "cargo" && reg.index_url) {
      lines.push(`index_url = ${q(reg.index_url)}`);
    }
    // `search_url`/`vuln_db_url` are three-state: absent (built-in default), a
    // URL, or `""` — the explicit way to switch the feature off.
    if (reg.search_url_disabled) {
      lines.push(`search_url = ""`);
    } else if (reg.search_url) {
      lines.push(`search_url = ${q(reg.search_url)}`);
    }
    if (reg.type === "goproxy") {
      if (reg.vuln_db_url_disabled) {
        lines.push(`vuln_db_url = ""`);
      } else if (reg.vuln_db_url) {
        lines.push(`vuln_db_url = ${q(reg.vuln_db_url)}`);
      }
    }
    const regHosts = csvToList(reg.hosts);
    if (regHosts.length) lines.push(`hosts = ${tomlArray(regHosts)}`);
    if (!reg.path_routing) lines.push(`path_routing = false`);
    if (isPathAddressed(reg)) {
      const allow = linesToList(reg.path_allow);
      if (allow.length) lines.push(`path_allow = ${tomlArray(allow)}`);
    }

    // [registries.rbac]
    lines.push("");
    lines.push("[registries.rbac]");
    lines.push(`anonymous = ${permsToToml(reg.rbac_anonymous)}`);
    lines.push(`user = ${permsToToml(reg.rbac_user)}`);
    lines.push(`admin = ${permsToToml(reg.rbac_admin)}`);
    // Both sub-tables must follow the scalar keys above — once a sub-table is
    // opened, any further `anonymous = …` would land inside it.
    const groups = reg.rbac_groups.filter((g) => g.name.trim());
    if (groups.length) {
      lines.push("");
      lines.push("[registries.rbac.groups]");
      for (const g of groups) {
        lines.push(`${q(g.name.trim())} = ${permsToToml(g.perms)}`);
      }
    }
    if (
      !reg.rbac_explore_anonymous ||
      !reg.rbac_explore_user ||
      !reg.rbac_explore_admin
    ) {
      lines.push("");
      lines.push("[registries.rbac.explore]");
      if (!reg.rbac_explore_anonymous) lines.push(`anonymous = false`);
      if (!reg.rbac_explore_user) lines.push(`user = false`);
      if (!reg.rbac_explore_admin) lines.push(`admin = false`);
    }

    // [registries.cache]
    const warmPackages = csvToList(reg.cache_warm_packages);
    const warmPaths = isPathAddressed(reg) ? linesToList(reg.cache_warm_paths) : [];
    const warmingOn = warmPackages.length > 0 || warmPaths.length > 0;
    const nonDefaultCache =
      reg.cache_metadata_ttl !== 300 ||
      reg.cache_artifact_ttl ||
      reg.cache_idle_days ||
      reg.cache_max_size_bytes ||
      reg.cache_keep_latest_n ||
      !reg.cache_serve_stale ||
      warmingOn;
    if (nonDefaultCache) {
      lines.push("");
      lines.push("[registries.cache]");
      if (reg.cache_metadata_ttl !== 300)
        lines.push(`metadata_ttl_secs = ${reg.cache_metadata_ttl}`);
      if (!reg.cache_serve_stale) lines.push(`serve_stale = false`);
      if (reg.cache_artifact_ttl)
        lines.push(`artifact_ttl_secs = ${reg.cache_artifact_ttl}`);
      if (reg.cache_idle_days) lines.push(`idle_days = ${reg.cache_idle_days}`);
      if (reg.cache_max_size_bytes)
        lines.push(`max_size_bytes = ${reg.cache_max_size_bytes}`);
      if (reg.cache_keep_latest_n)
        lines.push(`keep_latest_n = ${reg.cache_keep_latest_n}`);
      if (warmPackages.length)
        lines.push(`warm_packages = ${tomlArray(warmPackages)}`);
      if (warmPaths.length) lines.push(`warm_paths = ${tomlArray(warmPaths)}`);
      // Only meaningful once there is something to warm, so they stay out of the
      // file until then rather than pinning defaults for a disabled feature.
      if (warmingOn && reg.cache_warm_latest_n !== 1)
        lines.push(`warm_latest_n = ${reg.cache_warm_latest_n}`);
      if (warmingOn && reg.cache_warm_concurrency !== 2)
        lines.push(`warm_concurrency = ${reg.cache_warm_concurrency}`);
    }

    // [registries.rate_limit]
    if (reg.rate_limit_enabled) {
      lines.push("");
      lines.push("[registries.rate_limit]");
      lines.push(`requests_per_window = ${reg.rate_limit_rps}`);
      lines.push(`window_secs = ${reg.rate_limit_window}`);
      if (reg.rate_limit_enforcement !== "block")
        lines.push(`enforcement = ${q(reg.rate_limit_enforcement)}`);
      // Per-group overrides of the registry-wide budget above.
      for (const g of reg.rate_limit_groups) {
        if (!g.name.trim()) continue;
        lines.push("");
        lines.push("[[registries.rate_limit.groups]]");
        lines.push(`name = ${q(g.name.trim())}`);
        lines.push(`requests_per_window = ${g.requests_per_window}`);
        lines.push(`window_secs = ${g.window_secs}`);
        if (g.enforcement) lines.push(`enforcement = ${q(g.enforcement)}`);
      }
    }

    // [registries.quota]
    if (reg.quota_enabled && (reg.mode === "local" || reg.mode === "hybrid")) {
      lines.push("");
      lines.push("[registries.quota]");
      if (reg.quota_max_bytes)
        lines.push(`max_storage_bytes_per_user = ${reg.quota_max_bytes}`);
      if (reg.quota_max_packages)
        lines.push(`max_packages_per_user = ${reg.quota_max_packages}`);
      if (reg.quota_warn_threshold_pct !== 80)
        lines.push(`warn_threshold_pct = ${reg.quota_warn_threshold_pct}`);
      if (reg.quota_enforcement !== "block")
        lines.push(`enforcement = ${q(reg.quota_enforcement)}`);
    }

    // [registries.beta_channel]
    if (
      reg.beta_channel_enabled &&
      (reg.mode === "local" || reg.mode === "hybrid")
    ) {
      lines.push("");
      lines.push("[registries.beta_channel]");
      lines.push(`enabled = true`);
    }

    // [registries.versioning]
    if (
      reg.versioning_enabled &&
      (reg.mode === "local" || reg.mode === "hybrid")
    ) {
      lines.push("");
      lines.push("[registries.versioning]");
      if (reg.versioning_enforce_semver) lines.push(`enforce_semver = true`);
      if (!reg.versioning_allow_prerelease) lines.push(`allow_prerelease = false`);
      if (reg.versioning_pattern) lines.push(`version_pattern = ${q(reg.versioning_pattern)}`);
    }

    // [registries.signing]
    if (
      reg.signing_enabled &&
      (reg.mode === "local" || reg.mode === "hybrid")
    ) {
      lines.push("");
      lines.push("[registries.signing]");
      if (reg.signing_required) lines.push(`required = true`);
      if (reg.signing_allowed_types) {
        const types = reg.signing_allowed_types.split(",").map((t) => t.trim()).filter(Boolean);
        if (types.length) lines.push(`allowed_types = [${types.map(q).join(", ")}]`);
      }
      if (reg.signing_verify_on_download) lines.push(`verify_on_download = true`);
      const trustedKeys = csvToList(reg.signing_trusted_keys);
      if (trustedKeys.length)
        lines.push(`trusted_keys = ${tomlArray(trustedKeys)}`);
    }

    // [registries.repo_signing] — deb/rpm repository metadata signing
    if (reg.repo_signing_enabled && REPO_SIGNING_TYPES.has(reg.type)) {
      lines.push("");
      lines.push("[registries.repo_signing]");
      lines.push(`seed_hex = ${q(reg.repo_signing_seed_hex)}`);
      if (reg.repo_signing_user_id)
        lines.push(`user_id = ${q(reg.repo_signing_user_id)}`);
      if (reg.repo_signing_created)
        lines.push(`created = ${reg.repo_signing_created}`);
    }

    // [registries.sbom]
    if (reg.sbom_enabled) {
      lines.push("");
      lines.push("[registries.sbom]");
      lines.push(`enabled = true`);
      const formats = csvToList(reg.sbom_formats);
      if (formats.length) lines.push(`formats = ${tomlArray(formats)}`);
      if (reg.sbom_required) lines.push(`required = true`);
      if (!reg.sbom_fetch_upstream) lines.push(`fetch_upstream = false`);
    }

    // [registries.integrity] — omitted entirely unless the operator changed
    // something, since the absent section already means "verify and block on
    // mismatch, warn when no checksum is advertised".
    if (reg.integrity_customised) {
      lines.push("");
      lines.push("[registries.integrity]");
      if (!reg.integrity_enabled) lines.push(`enabled = false`);
      if (!reg.integrity_block_on_mismatch)
        lines.push(`block_on_mismatch = false`);
      if (reg.integrity_require_metadata) lines.push(`require_metadata = true`);
      if (reg.integrity_verify_on_serve) lines.push(`verify_on_serve = true`);
      if (reg.integrity_require_metadata) {
        const bypass = csvToList(reg.integrity_bypass_roles);
        if (bypass.length) lines.push(`bypass_roles = ${tomlArray(bypass)}`);
      }
    }

    // [[registries.rules]]
    const pushBypassRoles = (csv: string) => {
      const roles = csvToList(csv);
      if (roles.length) lines.push(`bypass_roles = [${roles.map(q).join(", ")}]`);
    };
    if (reg.rule_age_gate_enabled) {
      lines.push("");
      lines.push("[[registries.rules]]");
      lines.push(`kind = "release_age_gate"`);
      lines.push(`min_age_secs = ${reg.rule_age_gate_min_age}`);
      if (reg.rule_age_gate_deny_missing_timestamp)
        lines.push(`deny_missing_timestamp = true`);
      pushBypassRoles(reg.rule_age_gate_bypass_roles);
    }
    if (reg.rule_deny_latest_enabled) {
      lines.push("");
      lines.push("[[registries.rules]]");
      lines.push(`kind = "deny_latest"`);
      pushBypassRoles(reg.rule_deny_latest_bypass_roles);
    }
    if (reg.rule_signed_release_enabled) {
      lines.push("");
      lines.push("[[registries.rules]]");
      lines.push(`kind = "require_signed_release"`);
      lines.push(`enabled = true`);
      if (reg.rule_signed_release_deny_missing)
        lines.push(`deny_missing_signature = true`);
      pushBypassRoles(reg.rule_signed_release_bypass_roles);
    }
    if (reg.rule_license_gate_enabled) {
      lines.push("");
      lines.push("[[registries.rules]]");
      lines.push(`kind = "license_gate"`);
      const licAllow = csvToList(reg.rule_license_gate_allow);
      const licDeny = csvToList(reg.rule_license_gate_deny);
      if (licAllow.length) lines.push(`allow = ${tomlArray(licAllow)}`);
      if (licDeny.length) lines.push(`deny = ${tomlArray(licDeny)}`);
      if (!reg.rule_license_gate_allow_unknown)
        lines.push(`allow_unknown = false`);
      if (reg.rule_license_gate_block) lines.push(`block = true`);
      pushBypassRoles(reg.rule_license_gate_bypass_roles);
    }
    if (reg.rule_version_gate_enabled) {
      // Each entry may itself contain a comma (">=1.2.0, <2.0.0"), so these two
      // are one-per-line fields rather than comma-separated ones.
      const verAllow = linesToList(reg.rule_version_gate_allow);
      const verBlock = linesToList(reg.rule_version_gate_block);
      if (verAllow.length || verBlock.length) {
        lines.push("");
        lines.push("[[registries.rules]]");
        lines.push(`kind = "version_gate"`);
        if (verAllow.length) lines.push(`allow = ${tomlArray(verAllow)}`);
        if (verBlock.length) lines.push(`block = ${tomlArray(verBlock)}`);
        pushBypassRoles(reg.rule_version_gate_bypass_roles);
      }
    }
    if (reg.rule_cve_gate_enabled) {
      lines.push("");
      lines.push("[[registries.rules]]");
      lines.push(`kind = "cve_gate"`);
      lines.push(`min_severity = ${q(reg.rule_cve_gate_min_severity)}`);
      if (reg.rule_cve_gate_block) lines.push(`block = true`);
      pushBypassRoles(reg.rule_cve_gate_bypass_roles);
    }
    if (reg.rule_trusted_publisher_enabled) {
      const allow = csvToList(reg.rule_trusted_publisher_allow);
      if (allow.length) {
        lines.push("");
        lines.push("[[registries.rules]]");
        lines.push(`kind = "trusted_publisher"`);
        lines.push(`allow = [${allow.map(q).join(", ")}]`);
        pushBypassRoles(reg.rule_trusted_publisher_bypass_roles);
      }
    }

    // [registries.feature_flags]
    if (!reg.feature_flags_socket_badge) {
      lines.push("");
      lines.push("[registries.feature_flags]");
      lines.push(`socket_badge = false`);
    }

    // [registries.upstream_auth]
    if (reg.upstream_auth_type) {
      lines.push("");
      lines.push("[registries.upstream_auth]");
      lines.push(`type = ${q(reg.upstream_auth_type)}`);
      if (reg.upstream_auth_type === "bearer" && reg.upstream_auth_token) {
        lines.push(`token = ${q(reg.upstream_auth_token)}`);
      } else if (reg.upstream_auth_type === "basic") {
        if (reg.upstream_auth_username)
          lines.push(`username = ${q(reg.upstream_auth_username)}`);
        if (reg.upstream_auth_password)
          lines.push(`password = ${q(reg.upstream_auth_password)}`);
      } else if (reg.upstream_auth_type === "header") {
        if (reg.upstream_auth_header_name)
          lines.push(`name = ${q(reg.upstream_auth_header_name)}`);
        if (reg.upstream_auth_header_value)
          lines.push(`value = ${q(reg.upstream_auth_header_value)}`);
      }
    }

    // [registries.tls]
    if (reg.tls_ca_cert_path) {
      lines.push("");
      lines.push("[registries.tls]");
      lines.push(`ca_cert_path = ${q(reg.tls_ca_cert_path)}`);
    }

    // [registries.proxy] — overrides the global [proxy] for this registry only.
    if (reg.proxy_enabled && reg.proxy_url) {
      lines.push("");
      lines.push("[registries.proxy]");
      lines.push(`url = ${q(reg.proxy_url)}`);
      if (reg.proxy_username) lines.push(`username = ${q(reg.proxy_username)}`);
      if (reg.proxy_password) lines.push(`password = ${q(reg.proxy_password)}`);
      if (reg.proxy_no_proxy) lines.push(`no_proxy = ${q(reg.proxy_no_proxy)}`);
    }
  }

  // [ip_blocking]
  if (ipBlocking.value.enabled) {
    lines.push("");
    lines.push("[ip_blocking]");
    lines.push(`enabled = true`);
    lines.push(`violation_threshold = ${ipBlocking.value.violation_threshold}`);
    lines.push(
      `violation_window_secs = ${ipBlocking.value.violation_window_secs}`,
    );
    lines.push(`ban_duration_secs = ${ipBlocking.value.ban_duration_secs}`);
    lines.push(
      `trigger_on_status = ${numListToToml(ipBlocking.value.trigger_on_status)}`,
    );
  }

  // [vulnerability_scan]
  if (vulnerabilityScan.value.enabled) {
    lines.push("");
    lines.push("[vulnerability_scan]");
    lines.push(`enabled = true`);
    lines.push(`interval_secs = ${vulnerabilityScan.value.interval_secs}`);
    if (vulnerabilityScan.value.osv_api_url) {
      lines.push(`osv_api_url = ${q(vulnerabilityScan.value.osv_api_url)}`);
    }
    if (vulnerabilityScan.value.batch_size !== 100) {
      lines.push(`batch_size = ${vulnerabilityScan.value.batch_size}`);
    }
  }

  // [stats]
  if (
    !stats.value.history_enabled ||
    !stats.value.metrics_enabled ||
    stats.value.history_retention_days !== 30
  ) {
    lines.push("");
    lines.push("[stats]");
    if (!stats.value.history_enabled) lines.push(`history_enabled = false`);
    if (stats.value.history_retention_days !== 30)
      lines.push(`history_retention_days = ${stats.value.history_retention_days}`);
    if (!stats.value.metrics_enabled) lines.push(`metrics_enabled = false`);
  }

  // [subdomain_routing]
  if (subdomainRouting.value.enabled) {
    lines.push("");
    lines.push("[subdomain_routing]");
    lines.push(`enabled = true`);
    lines.push(`base_domain = ${q(subdomainRouting.value.base_domain)}`);
    if (subdomainRouting.value.scheme !== "https")
      lines.push(`scheme = ${q(subdomainRouting.value.scheme)}`);
  }

  // [proxy] — global egress proxy for every upstream fetch.
  if (upstreamProxy.value.enabled && upstreamProxy.value.url) {
    lines.push("");
    lines.push("[proxy]");
    lines.push(`url = ${q(upstreamProxy.value.url)}`);
    if (upstreamProxy.value.username)
      lines.push(`username = ${q(upstreamProxy.value.username)}`);
    if (upstreamProxy.value.password)
      lines.push(`password = ${q(upstreamProxy.value.password)}`);
    if (upstreamProxy.value.no_proxy)
      lines.push(`no_proxy = ${q(upstreamProxy.value.no_proxy)}`);
  }

  // [notifications]
  if (notifications.value.enabled) {
    lines.push("");
    lines.push("[notifications]");
    lines.push(`enabled = true`);
    for (const ch of notifications.value.channels) {
      if (!ch.name.trim()) continue;
      lines.push("");
      lines.push("[[notifications.channels]]");
      lines.push(`name = ${q(ch.name.trim())}`);
      lines.push(`type = ${q(ch.type)}`);
      if (ch.type === "email") {
        lines.push(`smtp_host = ${q(ch.smtp_host)}`);
        if (ch.smtp_port !== 587) lines.push(`smtp_port = ${ch.smtp_port}`);
        if (ch.smtp_user) lines.push(`smtp_user = ${q(ch.smtp_user)}`);
        if (ch.smtp_password) lines.push(`smtp_password = ${q(ch.smtp_password)}`);
        lines.push(`from = ${q(ch.from)}`);
        lines.push(`to = ${listToToml(ch.to)}`);
        if (!ch.tls) lines.push(`tls = false`);
      } else {
        lines.push(`url = ${q(ch.url)}`);
        // Only the generic webhook channel signs its payloads; Slack and Teams
        // authenticate by the secrecy of the hook URL itself.
        if (ch.type === "webhook" && ch.secret)
          lines.push(`secret = ${q(ch.secret)}`);
      }
      if (ch.timeout_secs !== 10) lines.push(`timeout_secs = ${ch.timeout_secs}`);
    }
    for (const hook of notifications.value.inbound) {
      if (!hook.name.trim()) continue;
      lines.push("");
      lines.push("[[notifications.inbound]]");
      lines.push(`name = ${q(hook.name.trim())}`);
      if (hook.secret) lines.push(`secret = ${q(hook.secret)}`);
    }
  }

  // [otel]
  if (otel.value.enabled) {
    lines.push("");
    lines.push("[otel]");
    lines.push(`endpoint = ${q(otel.value.endpoint)}`);
    lines.push(`service_name = ${q(otel.value.service_name)}`);
  }

  return lines.join("\n");
});

// ── Syntax highlighting ─────────────────────────────────────────────────────

function escHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function hlVal(val: string): string {
  if (!val) return "";
  if (val.startsWith('"') && val.endsWith('"') && val.length >= 2) {
    return `<span class="cg-hl-string">${escHtml(val)}</span>`;
  }
  if (val === "true" || val === "false") {
    return `<span class="cg-hl-bool">${val}</span>`;
  }
  if (/^-?\d+(\.\d+)?$/.test(val)) {
    return `<span class="cg-hl-number">${val}</span>`;
  }
  if (val.startsWith("[") && val.endsWith("]")) {
    const inner = val.slice(1, -1);
    let result = "";
    let last = 0;
    const strRe = /"([^"]*)"/g;
    let m: RegExpExecArray | null;
    while ((m = strRe.exec(inner)) !== null) {
      result += escHtml(inner.slice(last, m.index));
      result += `<span class="cg-hl-string">${escHtml(m[0])}</span>`;
      last = m.index + m[0].length;
    }
    result += escHtml(inner.slice(last));
    return `[${result}]`;
  }
  return escHtml(val);
}

const highlightedToml = computed(() =>
  toml.value
    .split("\n")
    .map((line) => {
      const trimmed = line.trimStart();
      const indent = escHtml(line.slice(0, line.length - trimmed.length));
      if (!trimmed) return "";
      if (trimmed.startsWith("#")) {
        return `<span class="cg-hl-comment">${escHtml(line)}</span>`;
      }
      const arrM = trimmed.match(/^(\[\[)([\w.]+)(\]\])$/);
      if (arrM) {
        return `${indent}<span class="cg-hl-bracket">[[</span><span class="cg-hl-table">${escHtml(arrM[2])}</span><span class="cg-hl-bracket">]]</span>`;
      }
      const tblM = trimmed.match(/^(\[)([\w.]+)(\])$/);
      if (tblM) {
        return `${indent}<span class="cg-hl-bracket">[</span><span class="cg-hl-table">${escHtml(tblM[2])}</span><span class="cg-hl-bracket">]</span>`;
      }
      const kvM = trimmed.match(/^([\w-]+)\s*=\s*(.+)$/);
      if (kvM) {
        return `${indent}<span class="cg-hl-key">${escHtml(kvM[1])}</span> <span class="cg-hl-eq">=</span> ${hlVal(kvM[2])}`;
      }
      return escHtml(line);
    })
    .join("\n"),
);

// ── Actions ─────────────────────────────────────────────────────────────────

const copied = ref(false);
async function copyToml() {
  await navigator.clipboard.writeText(toml.value);
  copied.value = true;
  setTimeout(() => {
    copied.value = false;
  }, 1500);
}

function downloadToml() {
  const blob = new Blob([toml.value], { type: "text/plain" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = "config.toml";
  a.click();
  URL.revokeObjectURL(a.href);
}

// Auth providers
function addAuthProvider() {
  authProviders.value.push(blankAuthProvider());
}
function removeAuthProvider(id: number) {
  authProviders.value = authProviders.value.filter((a) => a.id !== id);
}
function addToken(auth: AuthProvider) {
  auth.tokens.push({ id: tokenSeq++, value: "", role: "user", user_id: "" });
}
function removeToken(auth: AuthProvider, id: number) {
  auth.tokens = auth.tokens.filter((t) => t.id !== id);
}
function addActionsRule(auth: AuthProvider) {
  auth.actions_rules.push({
    id: ruleSeq++,
    group: "",
    group_template: "",
    role: "",
    match_mode: "all",
    conditions: [],
  });
}
function removeActionsRule(auth: AuthProvider, id: number) {
  auth.actions_rules = auth.actions_rules.filter((r) => r.id !== id);
}
function addCondition(rule: ActionsRule) {
  rule.conditions.push({ id: condSeq++, claim: "", pattern: "", match_type: "auto" });
}
function removeCondition(rule: ActionsRule, id: number) {
  rule.conditions = rule.conditions.filter((c) => c.id !== id);
}

function addRoleMapping(list: RoleMapping[]) {
  list.push({ id: mappingSeq++, claim: "", role: "user" });
}
function removeRoleMapping(auth: AuthProvider, key: "oidc" | "k8s", id: number) {
  if (key === "oidc") {
    auth.oidc_role_mappings = auth.oidc_role_mappings.filter((m) => m.id !== id);
  } else {
    auth.k8s_role_mappings = auth.k8s_role_mappings.filter((m) => m.id !== id);
  }
}

let groupSeq = 0;
function addRbacGroup(reg: Registry) {
  reg.rbac_groups.push({ id: groupSeq++, name: "", perms: "releases:read" });
}
function removeRbacGroup(reg: Registry, id: number) {
  reg.rbac_groups = reg.rbac_groups.filter((g) => g.id !== id);
}

let rlGroupSeq = 0;
function addRateLimitGroup(reg: Registry) {
  reg.rate_limit_groups.push({
    id: rlGroupSeq++,
    name: "",
    requests_per_window: reg.rate_limit_rps,
    window_secs: reg.rate_limit_window,
    enforcement: "",
  });
}
function removeRateLimitGroup(reg: Registry, id: number) {
  reg.rate_limit_groups = reg.rate_limit_groups.filter((g) => g.id !== id);
}

function addChannel() {
  notifications.value.channels.push({
    id: channelSeq++,
    type: "slack",
    name: "",
    url: "",
    secret: "",
    timeout_secs: 10,
    smtp_host: "",
    smtp_port: 587,
    smtp_user: "",
    smtp_password: "",
    from: "",
    to: "",
    tls: true,
  });
}
function removeChannel(id: number) {
  notifications.value.channels = notifications.value.channels.filter(
    (c) => c.id !== id,
  );
}
function addInboundHook() {
  notifications.value.inbound.push({ id: inboundSeq++, name: "", secret: "" });
}
function removeInboundHook(id: number) {
  notifications.value.inbound = notifications.value.inbound.filter(
    (h) => h.id !== id,
  );
}

function addRegistry() {
  registries.value.push(defaultRegistry("npm"));
}
function removeRegistry(id: number) {
  registries.value = registries.value.filter((r) => r.id !== id);
}
// Registry types that only support proxy mode (no private/local hosting) — they
// mirror the backend's local/hybrid allowlist: anything NOT in it is proxy-only.
const PROXY_ONLY_TYPES = new Set<RegistryType>([
  "github",
  "forgejo",
  "gitlab",
  "jetbrains",
  "generic",
]);
const isProxyOnly = (reg: Registry) => PROXY_ONLY_TYPES.has(reg.type);

function onTypeChange(reg: Registry) {
  reg.upstreams = defaultUpstream[reg.type];
  // Proxy-only types can't run in local/hybrid mode; force proxy.
  if (isProxyOnly(reg)) reg.mode = "proxy";
  // `path_allow` is rejected outright on kinds that aren't path-addressed, and
  // is mandatory on `generic` — so it follows the type rather than persisting
  // across a switch.
  if (!isPathAddressed(reg)) {
    reg.path_allow = "";
    reg.cache_warm_paths = "";
  } else if (reg.type === "generic" && !reg.path_allow.trim()) {
    reg.path_allow = "**";
  }
}

function addBackend() {
  storageBackends.value.push({
    id: backendSeq++,
    name: "",
    type: "filesystem",
    path: "./cache",
    bucket: "",
    region: "us-east-1",
    endpoint_url: "",
    force_path_style: false,
    prefix: "",
  });
}
function removeBackend(id: number) {
  storageBackends.value = storageBackends.value.filter((b) => b.id !== id);
}

const backendNames = computed(() =>
  storageBackends.value.map((b) => b.name).filter(Boolean),
);

// Registry types whose manifests the SBOM extractor can read a licence out of.
// Everywhere else the licence is permanently unknown, so a blocking gate that
// also refuses unknowns denies every download — the backend emits a
// `license-gate.denies-everything` warning for exactly this shape.
const LICENSE_AWARE_TYPES = new Set<RegistryType>([
  "cargo",
  "maven",
  "npm",
  "nuget",
  "pypi",
]);
const licenseGateDeniesEverything = (reg: Registry) =>
  reg.rule_license_gate_enabled &&
  reg.rule_license_gate_block &&
  !reg.rule_license_gate_allow_unknown &&
  !LICENSE_AWARE_TYPES.has(reg.type);

const isLocalOrHybrid = (reg: Registry) =>
  reg.mode === "local" || reg.mode === "hybrid";

function composerRepoSnippet(registryName: string): string {
  return `{
  "repositories": [
    {
      "type": "composer",
      "url": "https://your-batlehub-host/proxy/${registryName}/",
      "options": {
        "http": {
          "header": ["Authorization: Bearer <token>"]
        }
      }
    }
  ]
}`;
}

const composerAuthSnippet = `{
  "http-basic": {
    "your-batlehub-host": {
      "username": "user",
      "password": "<your-token>"
    }
  }
}`;
</script>

<template>
  <div class="cg-root">
    <!-- ── LEFT: form ──────────────────────────────────────────────────── -->
    <div class="cg-form">
      <!-- Server -->
      <section class="cg-section">
        <h3>Server</h3>
        <div class="cg-two-col">
          <label
            >Host<input v-model="server.host" placeholder="0.0.0.0"
          /></label>
          <label
            >Port<input
              v-model.number="server.port"
              type="number"
              min="1"
              max="65535"
          /></label>
        </div>
        <label
          >Static directory (optional)<input
            v-model="server.static_dir"
            placeholder="./ui/dist"
        /></label>
        <label
          >CLI binary directory (optional)<input
            v-model="server.cli_binary_path"
            placeholder="./dist/cli"
          /><span class="cg-field-hint"
            >Directory of pre-built <code>batlehub-cli</code> binaries served by
            the in-app CLI download page. Leave blank to disable it.</span
          ></label
        >
        <label
          >CORS allowed origins (optional, comma-separated)<input
            v-model="server.cors_allowed_origins"
            placeholder="https://batlehub.example.com"
          /><span class="cg-field-hint"
            >Leave blank to allow all origins (fine for development). Restrict in
            production.</span
          ></label
        >
        <label class="cg-check cg-mb">
          <input type="checkbox" v-model="server.trusted_proxies_set" />
          Declare a trusted-proxy policy
        </label>
        <label v-if="server.trusted_proxies_set"
          >Trusted proxy IPs (comma-separated)<input
            v-model="server.trusted_proxies"
            placeholder="10.0.0.1, 10.0.0.2"
          /><span class="cg-field-hint"
            >IPs of reverse proxies trusted to forward
            <code>X-Forwarded-For</code>. An empty list is a policy in itself —
            it means trust nobody and always use the TCP peer address. This
            supersedes the deprecated <code>[ip_blocking].trusted_proxies</code>,
            which logs a warning at startup.</span
          ></label
        >
      </section>

      <!-- Database -->
      <section class="cg-section">
        <h3>Database</h3>
        <label>
          PostgreSQL URL
          <input
            v-model="database.url"
            placeholder="postgresql://batlehub:changeme@localhost:5432/batlehub"
          />
        </label>
        <div class="cg-two-col">
          <label>
            Max connections
            <input
              v-model.number="database.max_connections"
              type="number"
              min="1"
            />
            <span class="cg-field-hint">Connection pool size (default: 10)</span>
          </label>
          <label>
            Min connections
            <input
              v-model.number="database.min_connections"
              type="number"
              min="0"
            />
            <span class="cg-field-hint">Idle connections kept warm (default: 1)</span>
          </label>
        </div>
        <label>
          Acquire timeout (s)
          <input
            v-model.number="database.acquire_timeout_secs"
            type="number"
            min="1"
          />
          <span class="cg-field-hint"
            >How long a request waits for a free connection before failing
            (default: 30)</span
          >
        </label>
      </section>

      <!-- Metadata Cache -->
      <section class="cg-section">
        <h3>Metadata Cache</h3>
        <div class="cg-radio-row cg-mb">
          <label class="cg-radio"
            ><input type="radio" v-model="metaCache.type" value="memory" />
            Memory</label
          >
          <label class="cg-radio"
            ><input type="radio" v-model="metaCache.type" value="postgres" />
            PostgreSQL</label
          >
          <label class="cg-radio"
            ><input type="radio" v-model="metaCache.type" value="redis" />
            Redis</label
          >
        </div>
        <span class="cg-field-hint">
          <template v-if="metaCache.type === 'memory'"
            >In-process cache — fast but lost on restart. Good for single-node
            dev deployments.</template
          >
          <template v-else-if="metaCache.type === 'postgres'"
            >Persisted in the <code>metadata_cache</code> table — survives
            restarts, shared across replicas.</template
          >
          <template v-else
            >Persisted in Redis — survives restarts, shared across
            replicas.</template
          >
        </span>
        <template v-if="metaCache.type === 'redis'">
          <label style="margin-top: 0.5rem"
            >Redis URL<input
              v-model="metaCache.url"
              placeholder="redis://localhost:6379"
          /></label>
        </template>
      </section>

      <!-- Limits -->
      <section class="cg-section">
        <h3>Limits</h3>
        <label>
          Max artifact size (bytes)
          <input
            v-model="limits.max_artifact_size_bytes"
            placeholder="524288000  (500 MiB default)"
          />
          <span class="cg-field-hint"
            >Applies to both proxy downloads and local publishes. Leave blank to
            use the 500 MiB default.</span
          >
        </label>
      </section>

      <!-- Storage -->
      <section class="cg-section">
        <h3>Storage</h3>
        <div class="cg-radio-row cg-mb">
          <label class="cg-radio"
            ><input type="radio" v-model="storageMode" value="single" /> Single
            backend</label
          >
          <label class="cg-radio"
            ><input type="radio" v-model="storageMode" value="multi" />
            Multi-backend</label
          >
        </div>

        <!-- Single backend -->
        <template v-if="storageMode === 'single'">
          <div class="cg-radio-row cg-mb">
            <label class="cg-radio"
              ><input
                type="radio"
                v-model="singleStorage.type"
                value="filesystem"
              />
              Filesystem</label
            >
            <label class="cg-radio"
              ><input type="radio" v-model="singleStorage.type" value="s3" /> S3
              / RustFS</label
            >
          </div>
          <template v-if="singleStorage.type === 'filesystem'">
            <label
              >Cache path<input
                v-model="singleStorage.path"
                placeholder="./cache"
            /></label>
          </template>
          <template v-else>
            <div class="cg-two-col">
              <label
                >Bucket<input
                  v-model="singleStorage.bucket"
                  placeholder="my-artifacts"
              /></label>
              <label
                >Region<input
                  v-model="singleStorage.region"
                  placeholder="us-east-1"
              /></label>
            </div>
            <label
              >Endpoint URL (optional)<input
                v-model="singleStorage.endpoint_url"
                placeholder="http://minio:9000"
            /></label>
            <label
              >Key prefix (optional)<input
                v-model="singleStorage.prefix"
                placeholder="batlehub/"
              /><span class="cg-field-hint"
                >Prepended to every object key — acts as a folder inside the
                bucket.</span
              ></label
            >
            <label class="cg-check cg-mb">
              <input type="checkbox" v-model="singleStorage.force_path_style" />
              Force path-style URLs (required for MinIO, RustFS)
            </label>
          </template>
        </template>

        <!-- Multi-backend -->
        <template v-else>
          <label
            >Default backend name<input
              v-model="storageDefault"
              placeholder="primary"
          /></label>
          <div v-for="b in storageBackends" :key="b.id" class="cg-list-item">
            <div class="cg-two-col">
              <label
                >Backend name<input v-model="b.name" placeholder="primary"
              /></label>
              <label>
                Type
                <select v-model="b.type">
                  <option value="filesystem">Filesystem</option>
                  <option value="s3">S3 / RustFS</option>
                </select>
              </label>
            </div>
            <template v-if="b.type === 'filesystem'">
              <label
                >Cache path<input v-model="b.path" placeholder="./cache"
              /></label>
            </template>
            <template v-else>
              <div class="cg-two-col">
                <label
                  >Bucket<input v-model="b.bucket" placeholder="my-artifacts"
                /></label>
                <label
                  >Region<input v-model="b.region" placeholder="us-east-1"
                /></label>
              </div>
              <label
                >Endpoint URL (optional)<input
                  v-model="b.endpoint_url"
                  placeholder="http://minio:9000"
              /></label>
              <label
                >Key prefix (optional)<input
                  v-model="b.prefix"
                  placeholder="batlehub/"
                /><span class="cg-field-hint"
                  >Prepended to every object key.</span
                ></label
              >
              <label class="cg-check cg-mb">
                <input type="checkbox" v-model="b.force_path_style" />
                Force path-style URLs (required for MinIO, RustFS)
              </label>
            </template>
            <button class="cg-btn-remove" @click="removeBackend(b.id)">
              Remove
            </button>
          </div>
          <button class="cg-btn-add" @click="addBackend">+ Add backend</button>
        </template>
      </section>

      <!-- Auth providers -->
      <section class="cg-section">
        <h3>Authentication</h3>
        <div v-for="auth in authProviders" :key="auth.id" class="cg-list-item">
          <label>
            Provider type
            <select v-model="auth.type">
              <option value="token">Static tokens</option>
              <option value="oidc">OIDC / OAuth2</option>
              <option value="kubernetes">Kubernetes service accounts</option>
              <option value="actions-oidc">GitHub Actions OIDC</option>
            </select>
          </label>

          <!-- Token auth -->
          <template v-if="auth.type === 'token'">
            <div v-for="tok in auth.tokens" :key="tok.id" class="cg-subitem">
              <label>
                Token value
                <span class="cg-label-note"
                  >(raw — an Argon2id hash is written to the config)</span
                >
                <input
                  v-model="tok.value"
                  placeholder="my-secret-token"
                  autocomplete="off"
                />
              </label>
              <div v-if="tok.value.trim()" class="cg-hash-status">
                <template v-if="tokenHashes[tok.id] === null">
                  <span class="cg-hash-computing">⏳ Computing Argon2id hash…</span>
                </template>
                <template
                  v-else-if="tokenHashes[tok.id]?.startsWith('$argon2')"
                >
                  <span class="cg-hash-ready"
                    >🔐 Argon2id hash ready &mdash; use the raw value above with
                    <code>Authorization: Bearer &lt;raw token&gt;</code></span
                  >
                </template>
                <template v-else>
                  <span class="cg-hash-warn"
                    >⚠ Browser hashing unavailable &mdash; after download run
                    <code>batlehub hash-token {{ tok.value }}</code> and replace
                    the value in the config</span
                  >
                </template>
              </div>
              <div class="cg-two-col">
                <label>
                  Role
                  <select v-model="tok.role">
                    <option value="admin">admin</option>
                    <option value="user">user</option>
                    <option value="anonymous">anonymous</option>
                  </select>
                </label>
                <label
                  >User ID (optional)<input
                    v-model="tok.user_id"
                    placeholder="alice"
                /></label>
              </div>
              <button class="cg-btn-remove" @click="removeToken(auth, tok.id)">
                Remove token
              </button>
            </div>
          </template>

          <!-- OIDC auth -->
          <template v-else-if="auth.type === 'oidc'">
            <label
              >Provider name (optional)<input
                v-model="auth.oidc_name"
                placeholder="oidc"
              /><span class="cg-field-hint"
                >Used as the group prefix (e.g. <code>oidc:team-a</code>). Only
                needed when running multiple OIDC providers.</span
              ></label
            >
            <label
              >Issuer URL<input
                v-model="auth.oidc_issuer"
                placeholder="https://accounts.example.com"
            /></label>
            <div class="cg-two-col">
              <label
                >Client ID<input
                  v-model="auth.oidc_client_id"
                  placeholder="batlehub"
              /></label>
              <label
                >Client secret<input
                  v-model="auth.oidc_client_secret"
                  type="password"
                  placeholder="(optional for PKCE)"
              /></label>
            </div>
            <label
              >Redirect URI<input
                v-model="auth.oidc_redirect_uri"
                placeholder="https://batlehub.example.com/api/v1/auth/oidc/callback"
            /></label>
            <label
              >Frontend URL (dev only)<input
                v-model="auth.oidc_frontend_url"
                placeholder="http://localhost:5173"
              /><span class="cg-field-hint"
                >Leave blank in production — the callback redirects to the same
                origin.</span
              ></label
            >
            <div class="cg-two-col">
              <label
                >User ID claim<input
                  v-model="auth.oidc_user_id_claim"
                  placeholder="sub"
              /></label>
              <label
                >Role claim<input
                  v-model="auth.oidc_role_claim"
                  placeholder="role"
              /></label>
            </div>
            <label
              >Scopes (optional, comma-separated)<input
                v-model="auth.oidc_scopes"
                placeholder="openid, profile, email"
              /><span class="cg-field-hint"
                >Defaults to <code>openid, profile, email</code> when
                blank.</span
              ></label
            >

            <p class="cg-subsection-label" style="margin-top: 0.75rem">
              Role mappings
            </p>
            <span
              class="cg-field-hint"
              style="margin-bottom: 0.5rem; display: block"
              >Maps values of the <code>{{ auth.oidc_role_claim || "role" }}</code>
              claim to proxy roles. <strong>A claim value with no entry here
              falls back to <code>anonymous</code></strong> — without at least
              one <code>admin</code> mapping nobody who logs in through this
              provider can administer the server.</span
            >
            <div
              v-for="m in auth.oidc_role_mappings"
              :key="m.id"
              class="cg-condition-item"
            >
              <div class="cg-two-col">
                <label
                  >Claim value<input
                    v-model="m.claim"
                    placeholder="batlehub-admins"
                /></label>
                <label>
                  Role
                  <select v-model="m.role">
                    <option value="admin">admin</option>
                    <option value="user">user</option>
                    <option value="anonymous">anonymous</option>
                  </select>
                </label>
              </div>
              <button
                class="cg-btn-remove"
                @click="removeRoleMapping(auth, 'oidc', m.id)"
              >
                Remove mapping
              </button>
            </div>
            <button
              class="cg-btn-add"
              @click="addRoleMapping(auth.oidc_role_mappings)"
            >
              + Add role mapping
            </button>
          </template>

          <!-- Kubernetes auth -->
          <template v-else-if="auth.type === 'kubernetes'">
            <label
              >Provider name (optional)<input
                v-model="auth.k8s_name"
                placeholder="kubernetes"
              /><span class="cg-field-hint"
                >Used as the group prefix (e.g.
                <code>kubernetes:ops</code>).</span
              ></label
            >
            <label
              >API server URL (optional)<input
                v-model="auth.k8s_api_server"
                placeholder="https://kubernetes.default.svc"
              /><span class="cg-field-hint"
                >Leave blank to use the in-cluster environment variables.</span
              ></label
            >
            <label
              >CA cert path (optional)<input
                v-model="auth.k8s_ca_cert_path"
                placeholder="/var/run/secrets/kubernetes.io/serviceaccount/ca.crt"
              /><span class="cg-field-hint"
                >Defaults to the standard in-cluster CA mount.</span
              ></label
            >
            <label
              >Service account token path (optional)<input
                v-model="auth.k8s_token_path"
                placeholder="/var/run/secrets/kubernetes.io/serviceaccount/token"
              /><span class="cg-field-hint"
                >Defaults to the standard in-cluster token mount.</span
              ></label
            >
            <label
              >Audiences (comma-separated)<input
                v-model="auth.k8s_audiences"
                placeholder="batlehub"
            /></label>

            <p class="cg-subsection-label" style="margin-top: 0.75rem">
              Role mappings
            </p>
            <span
              class="cg-field-hint"
              style="margin-bottom: 0.5rem; display: block"
              >Keys are Kubernetes usernames
              (<code>system:serviceaccount:&lt;ns&gt;:&lt;name&gt;</code>) or
              group names. <strong>An identity with no entry here falls back to
              <code>anonymous</code>.</strong></span
            >
            <div
              v-for="m in auth.k8s_role_mappings"
              :key="m.id"
              class="cg-condition-item"
            >
              <div class="cg-two-col">
                <label
                  >Username or group<input
                    v-model="m.claim"
                    placeholder="system:serviceaccount:ci:builder"
                /></label>
                <label>
                  Role
                  <select v-model="m.role">
                    <option value="admin">admin</option>
                    <option value="user">user</option>
                    <option value="anonymous">anonymous</option>
                  </select>
                </label>
              </div>
              <button
                class="cg-btn-remove"
                @click="removeRoleMapping(auth, 'k8s', m.id)"
              >
                Remove mapping
              </button>
            </div>
            <button
              class="cg-btn-add"
              @click="addRoleMapping(auth.k8s_role_mappings)"
            >
              + Add role mapping
            </button>
          </template>

          <!-- GitHub Actions OIDC auth -->
          <template v-else-if="auth.type === 'actions-oidc'">
            <label
              >Provider name (optional)<input
                v-model="auth.actions_name"
                placeholder="actions-oidc"
              /><span class="cg-field-hint"
                >Used as the group prefix in RBAC group rules.</span
              ></label
            >
            <label
              >Issuer URL<input
                v-model="auth.actions_issuer"
                placeholder="https://token.actions.githubusercontent.com"
              /><span class="cg-field-hint"
                >For GitHub.com use
                <code>https://token.actions.githubusercontent.com</code>.</span
              ></label
            >
            <label
              >User ID claim (optional)<input
                v-model="auth.actions_user_id_claim"
                placeholder="sub"
              /><span class="cg-field-hint"
                >JWT claim used as the user identifier. Defaults to
                <code>sub</code>.</span
              ></label
            >

            <!-- Rules -->
            <p class="cg-subsection-label" style="margin-top: 0.75rem">
              Rules
            </p>
            <span class="cg-field-hint" style="margin-bottom: 0.5rem; display: block"
              >Each rule assigns a role when a workflow token matches the given
              conditions.</span
            >
            <div
              v-for="rule in auth.actions_rules"
              :key="rule.id"
              class="cg-subitem"
            >
              <div class="cg-two-col">
                <label
                  >Group name (optional)<input
                    v-model="rule.group"
                    placeholder="ci-bots"
                  /><span class="cg-field-hint"
                    >Static group name assigned to matching tokens.</span
                  ></label
                >
                <label>
                  Role (optional)
                  <select v-model="rule.role">
                    <option value="">— none (group only) —</option>
                    <option value="admin">admin</option>
                    <option value="user">user</option>
                    <option value="anonymous">anonymous</option>
                  </select>
                  <span class="cg-field-hint">When blank the rule assigns groups without affecting role elevation.</span>
                </label>
              </div>
              <label
                >Group template (optional)<input
                  v-model="rule.group_template"
                  placeholder="{name}/{repository}/{ref_name}"
                /><span class="cg-field-hint"
                  >Template rendered from JWT claims.
                  <code>{repository}</code>, <code>{ref_name}</code> and any
                  other claim key are supported. Slashes are replaced with
                  dashes.</span
                ></label
              >
              <label>
                Condition match mode
                <select v-model="rule.match_mode">
                  <option value="all">all (every condition must pass)</option>
                  <option value="any">any (at least one must pass)</option>
                </select>
              </label>

              <!-- Conditions -->
              <p class="cg-subsection-label">Conditions</p>
              <div
                v-for="cond in rule.conditions"
                :key="cond.id"
                class="cg-condition-item"
              >
                <div class="cg-two-col">
                  <label
                    >JWT claim<input
                      v-model="cond.claim"
                      placeholder="repository"
                  /></label>
                  <label
                    >Pattern<input
                      v-model="cond.pattern"
                      placeholder="myorg/*"
                  /></label>
                </div>
                <label>
                  Match type
                  <select v-model="cond.match_type">
                    <option value="auto">auto (glob if * present, else exact)</option>
                    <option value="glob">glob</option>
                    <option value="regex">regex</option>
                  </select>
                </label>
                <button
                  class="cg-btn-remove"
                  @click="removeCondition(rule, cond.id)"
                >
                  Remove condition
                </button>
              </div>
              <button class="cg-btn-add" @click="addCondition(rule)">
                + Add condition
              </button>
              <div style="margin-top: 0.5rem">
                <button
                  class="cg-btn-remove"
                  @click="removeActionsRule(auth, rule.id)"
                >
                  Remove rule
                </button>
              </div>
            </div>
            <button class="cg-btn-add" @click="addActionsRule(auth)">
              + Add rule
            </button>
          </template>

          <div class="cg-provider-actions">
            <button
              v-if="auth.type === 'token'"
              class="cg-btn-add"
              @click="addToken(auth)"
            >
              + Add token
            </button>
            <span v-else />
            <button class="cg-btn-remove" @click="removeAuthProvider(auth.id)">
              Remove provider
            </button>
          </div>
        </div>
        <button class="cg-btn-add" @click="addAuthProvider">
          + Add auth provider
        </button>
      </section>

      <!-- Registries -->
      <section class="cg-section">
        <h3>Registries</h3>
        <div v-for="reg in registries" :key="reg.id" class="cg-list-item">
          <div class="cg-two-col">
            <label>Name<input v-model="reg.name" placeholder="npm" /></label>
            <label>
              Type
              <select v-model="reg.type" @change="onTypeChange(reg)">
                <option value="npm">npm</option>
                <option value="cargo">Cargo</option>
                <option value="maven">Maven</option>
                <option value="rubygems">RubyGems</option>
                <option value="composer">Composer (PHP)</option>
                <option value="pypi">PyPI (Python)</option>
                <option value="conda">Conda</option>
                <option value="nuget">NuGet (.NET)</option>
                <option value="openvsx">OpenVSX</option>
                <option value="vscode-marketplace">VS Code Marketplace</option>
                <option value="goproxy">Go Modules</option>
                <option value="terraform">Terraform</option>
                <option value="github">GitHub</option>
                <option value="forgejo">Forgejo / Gitea</option>
                <option value="gitlab">GitLab</option>
                <option value="deb">Deb (APT)</option>
                <option value="rpm">RPM (YUM/DNF)</option>
                <option value="pacman">Pacman (Arch)</option>
                <option value="jetbrains">JetBrains IDE</option>
                <option value="jetbrains-marketplace">JetBrains Marketplace</option>
                <option value="generic">Generic (raw file mirror)</option>
              </select>
            </label>
          </div>
          <div class="cg-radio-row cg-mb">
            <label class="cg-radio"
              ><input type="radio" v-model="reg.mode" value="proxy" />
              proxy</label
            >
            <label class="cg-radio" :class="{ 'cg-disabled': isProxyOnly(reg) }"
              ><input
                type="radio"
                v-model="reg.mode"
                value="local"
                :disabled="isProxyOnly(reg)"
              />
              local</label
            >
            <label class="cg-radio" :class="{ 'cg-disabled': isProxyOnly(reg) }"
              ><input
                type="radio"
                v-model="reg.mode"
                value="hybrid"
                :disabled="isProxyOnly(reg)"
              />
              hybrid</label
            >
          </div>
          <p v-if="isProxyOnly(reg)" class="cg-hint">
            {{ reg.type }} is a proxy-only registry (no private hosting), so mode
            is locked to <code>proxy</code>.
          </p>
          <label v-if="reg.mode !== 'local'">
            Upstreams (one per line)
            <textarea v-model="reg.upstreams" rows="2" />
          </label>

          <!-- Composer client config hint -->
          <div v-if="reg.type === 'composer'" class="cg-registry-hint">
            <p class="cg-hint-title">Composer client setup</p>
            <p class="cg-hint-text">
              Add a repository entry to your project's
              <code>composer.json</code>:
            </p>
            <pre class="cg-hint-code">{{ composerRepoSnippet(reg.name) }}</pre>
            <p class="cg-hint-text" style="margin-top: 0.5rem">
              Store credentials in <code>auth.json</code> (never commit this
              file):
            </p>
            <pre class="cg-hint-code">{{ composerAuthSnippet }}</pre>
            <p class="cg-hint-text" style="margin-top: 0.5rem">
              Publish via ZIP upload (must contain
              <code>composer.json</code> with <code>"name"</code> and
              <code>"version"</code>):
            </p>
            <pre class="cg-hint-code">
curl -X POST \
  -H "Authorization: Bearer &lt;token&gt;" \
  -H "Content-Type: application/zip" \
  --data-binary @vendor-pkg-1.0.0.zip \
  "/proxy/{{ reg.name }}/api/upload"</pre
            >
          </div>

          <!-- PyPI client config hint -->
          <div v-if="reg.type === 'pypi'" class="cg-registry-hint">
            <p class="cg-hint-title">PyPI client setup</p>
            <p class="cg-hint-text">
              Point pip at the proxy via <code>~/.pip/pip.conf</code>:
            </p>
            <pre class="cg-hint-code">[global]
index-url = https://your-batlehub-host/proxy/{{ reg.name }}/simple/</pre>
            <p class="cg-hint-text" style="margin-top: 0.5rem">
              Or with uv in <code>pyproject.toml</code>:
            </p>
            <pre class="cg-hint-code">[[tool.uv.index]]
name = "batlehub"
url = "https://your-batlehub-host/proxy/{{ reg.name }}/simple/"
default = true</pre>
            <p class="cg-hint-text" style="margin-top: 0.5rem" v-if="isLocalOrHybrid(reg)">
              Publish with twine (local/hybrid mode):
            </p>
            <pre class="cg-hint-code" v-if="isLocalOrHybrid(reg)">
twine upload \
  --repository-url https://your-batlehub-host/proxy/{{ reg.name }}/legacy/ \
  --username __token__ --password &lt;your-token&gt; \
  dist/*</pre>
          </div>

          <!-- Conda client config hint -->
          <div v-if="reg.type === 'conda'" class="cg-registry-hint">
            <p class="cg-hint-title">Conda client setup</p>
            <p class="cg-hint-text">
              Add the proxy as a channel in <code>~/.condarc</code>:
            </p>
            <pre class="cg-hint-code">channels:
  - https://your-batlehub-host/proxy/{{ reg.name }}
  - nodefaults</pre>
            <p class="cg-hint-text" style="margin-top: 0.5rem">
              Credentials are read from <code>~/.netrc</code> automatically.
            </p>
            <p class="cg-hint-text" style="margin-top: 0.5rem" v-if="isLocalOrHybrid(reg)">
              Publish a package (local/hybrid mode):
            </p>
            <pre class="cg-hint-code" v-if="isLocalOrHybrid(reg)">
curl -X POST \
  -H "Authorization: Bearer &lt;token&gt;" \
  -H "Content-Type: application/octet-stream" \
  --data-binary @pkg-1.0.0-py311h0_0.tar.bz2 \
  "https://your-batlehub-host/proxy/{{ reg.name }}/linux-64/"</pre>
          </div>

          <!-- NuGet client config hint -->
          <div v-if="reg.type === 'nuget'" class="cg-registry-hint">
            <p class="cg-hint-title">NuGet client setup</p>
            <p class="cg-hint-text">
              Add the proxy as a NuGet source:
            </p>
            <pre class="cg-hint-code">dotnet nuget add source \
  https://your-batlehub-host/proxy/{{ reg.name }}/nuget/v3/index.json \
  --name {{ reg.name }}</pre>
            <p class="cg-hint-text" style="margin-top: 0.5rem">
              Or add to <code>nuget.config</code>:
            </p>
            <pre class="cg-hint-code">&lt;configuration&gt;
  &lt;packageSources&gt;
    &lt;add key="{{ reg.name }}" value="https://your-batlehub-host/proxy/{{ reg.name }}/nuget/v3/index.json" /&gt;
  &lt;/packageSources&gt;
&lt;/configuration&gt;</pre>
            <p class="cg-hint-text" style="margin-top: 0.5rem" v-if="isLocalOrHybrid(reg)">
              Publish a package (local/hybrid mode):
            </p>
            <pre class="cg-hint-code" v-if="isLocalOrHybrid(reg)">dotnet nuget push MyLib.1.0.0.nupkg \
  --api-key &lt;your-token&gt; \
  --source https://your-batlehub-host/proxy/{{ reg.name }}/nuget/v3/index.json</pre>
          </div>

          <label v-if="storageMode === 'multi'">
            Storage backend (blank = use default)
            <select v-model="reg.storage_backend">
              <option value="">— default ({{ storageDefault }}) —</option>
              <option v-for="n in backendNames" :key="n" :value="n">
                {{ n }}
              </option>
            </select>
          </label>
          <p class="cg-perm-label">
            Permissions
            <span class="cg-perm-hint"
              >(comma-separated; use <code>*</code> for all)</span
            >
          </p>
          <div class="cg-three-col">
            <label
              >anonymous<input v-model="reg.rbac_anonymous" placeholder=""
            /></label>
            <label
              >user<input
                v-model="reg.rbac_user"
                placeholder="releases:read, source:read"
            /></label>
            <label
              >admin<input v-model="reg.rbac_admin" placeholder="*"
            /></label>
          </div>

          <!-- RBAC groups -->
          <div
            v-for="g in reg.rbac_groups"
            :key="g.id"
            class="cg-condition-item"
          >
            <div class="cg-two-col">
              <label
                >Group name<input
                  v-model="g.name"
                  placeholder="oidc:team-a"
                /><span class="cg-field-hint"
                  >Prefixed with the provider name; use
                  <code>*:team-a</code> to match the group from any
                  provider.</span
                ></label
              >
              <label
                >Permissions<input
                  v-model="g.perms"
                  placeholder="releases:read, releases:write"
              /></label>
            </div>
            <button class="cg-btn-remove" @click="removeRbacGroup(reg, g.id)">
              Remove group
            </button>
          </div>
          <button class="cg-btn-add" @click="addRbacGroup(reg)">
            + Add group permission
          </button>

          <!-- Advanced toggle + remove row -->
          <div class="cg-registry-actions">
            <button
              class="cg-btn-advanced"
              @click="reg.showAdvanced = !reg.showAdvanced"
            >
              {{ reg.showAdvanced ? "▲ Hide advanced" : "▼ Advanced options" }}
            </button>
            <button class="cg-btn-remove" @click="removeRegistry(reg.id)">
              Remove registry
            </button>
          </div>

          <div v-if="reg.showAdvanced" class="cg-advanced">
            <!-- Routing & addressing -->
            <p class="cg-subsection-label">Routing &amp; addressing</p>
            <label
              >Vanity hosts (optional, comma-separated)<input
                v-model="reg.hosts"
                placeholder="npm.acme.io"
              /><span class="cg-field-hint"
                >Hostnames whose <em>root</em> serves this registry, in addition
                to <code>/proxy/{{ reg.name }}/…</code>. DNS and TLS for them are
                yours to arrange.</span
              ></label
            >
            <label class="cg-check cg-mb">
              <input type="checkbox" v-model="reg.path_routing" />
              Also serve under <code>/proxy/{{ reg.name }}/…</code>
            </label>
            <span
              v-if="!reg.path_routing"
              class="cg-field-hint"
              style="display: block; margin-bottom: 0.5rem"
              >The subpath returns <code>404</code> — a disabled ingress should
              look absent, not forbidden. Give the registry at least one host
              above (or enable subdomain routing) or nothing can reach it.</span
            >
            <template v-if="isPathAddressed(reg)">
              <label
                >Allowed upstream paths (one glob per line)
                <textarea v-model="reg.path_allow" rows="3" />
                <span class="cg-field-hint"
                  >Matched against the upstream-relative path, where
                  <code>*</code> also crosses <code>/</code>. Use
                  <code>**</code> to mirror everything.
                  <template v-if="reg.type === 'generic'"
                    ><strong>Required for generic registries</strong> — the
                    server refuses to start without it.</template
                  ></span
                >
              </label>
            </template>
            <label v-if="reg.type === 'cargo'"
              >Sparse index URL (optional)<input
                v-model="reg.index_url"
                placeholder="https://index.crates.io"
              /><span class="cg-field-hint"
                >Set for self-hosted Cargo registries (e.g. a Forgejo package
                feed).</span
              ></label
            >
            <label class="cg-check">
              <input type="checkbox" v-model="reg.search_url_disabled" />
              Disable upstream search for this registry
            </label>
            <label v-if="!reg.search_url_disabled"
              >Search URL (optional)<input
                v-model="reg.search_url"
                placeholder="https://search.maven.org"
              /><span class="cg-field-hint"
                >Overrides the built-in default (Maven and Composer have one;
                other types ignore this).</span
              ></label
            >
            <template v-if="reg.type === 'goproxy'">
              <label class="cg-check">
                <input type="checkbox" v-model="reg.vuln_db_url_disabled" />
                Disable the Go vulnerability database passthrough
              </label>
              <label v-if="!reg.vuln_db_url_disabled"
                >govulndb URL (optional)<input
                  v-model="reg.vuln_db_url"
                  placeholder="https://vuln.go.dev"
              /></label>
            </template>

            <!-- Package explorer visibility -->
            <p class="cg-subsection-label">Package explorer</p>
            <span class="cg-field-hint" style="display: block; margin-bottom: 0.5rem"
              >Who may browse this registry's cached package list in the UI.
              Independent of download permissions — all three are on by
              default.</span
            >
            <label class="cg-check">
              <input type="checkbox" v-model="reg.rbac_explore_anonymous" />
              anonymous
            </label>
            <label class="cg-check">
              <input type="checkbox" v-model="reg.rbac_explore_user" /> user
            </label>
            <label class="cg-check cg-mb">
              <input type="checkbox" v-model="reg.rbac_explore_admin" /> admin
            </label>

            <!-- Firewall mode -->
            <p class="cg-subsection-label">Firewall</p>
            <label class="cg-check cg-mb">
              <input type="checkbox" v-model="reg.firewall_only" />
              Firewall-only mode (enforce rules without caching)
            </label>
            <span v-if="reg.firewall_only" class="cg-field-hint" style="display: block; margin-bottom: 0.5rem"
              >Rules are evaluated but nothing is written to storage. Requests
              stream directly from upstream.</span
            >

            <!-- Cache policy -->
            <p class="cg-subsection-label">Cache policy</p>
            <div class="cg-two-col">
              <label
                >Metadata TTL (s)<input
                  v-model.number="reg.cache_metadata_ttl"
                  type="number"
                  min="0"
                /><span class="cg-field-hint">Default: 300 s</span></label
              >
              <label
                >Artifact TTL (s)<input
                  v-model="reg.cache_artifact_ttl"
                  placeholder="never"
                /><span class="cg-field-hint"
                  >Leave blank to keep forever</span
                ></label
              >
            </div>
            <div class="cg-two-col">
              <label
                >Idle eviction (days)<input
                  v-model="reg.cache_idle_days"
                  placeholder="never"
              /></label>
              <label
                >Size cap (bytes)<input
                  v-model="reg.cache_max_size_bytes"
                  placeholder="no cap"
              /></label>
            </div>
            <label
              >Keep latest N versions<input
                v-model="reg.cache_keep_latest_n"
                placeholder="keep all"
            /></label>
            <label class="cg-check cg-mb">
              <input type="checkbox" v-model="reg.cache_serve_stale" />
              Serve stale metadata when the upstream is unreachable
            </label>
            <span
              v-if="!reg.cache_serve_stale"
              class="cg-field-hint"
              style="display: block; margin-bottom: 0.5rem"
              >An upstream outage will surface as an error instead of an
              expired-but-usable answer.</span
            >

            <!-- Cache warming -->
            <p class="cg-subsection-label">Cache warming</p>
            <label
              >Packages to keep warm (comma-separated)<input
                v-model="reg.cache_warm_packages"
                placeholder="express, lodash"
              /><span class="cg-field-hint"
                >Fetched ahead of the first request so a cold cache never costs a
                user a round trip upstream.</span
              ></label
            >
            <label v-if="isPathAddressed(reg)"
              >Paths to keep warm (one per line)
              <textarea v-model="reg.cache_warm_paths" rows="2" />
              <span class="cg-field-hint"
                >Path-addressed registries warm by upstream path rather than
                package name.</span
              >
            </label>
            <div class="cg-two-col">
              <label
                >Warm latest N versions<input
                  v-model.number="reg.cache_warm_latest_n"
                  type="number"
                  min="1"
                /><span class="cg-field-hint">Default: 1</span></label
              >
              <label
                >Warm concurrency<input
                  v-model.number="reg.cache_warm_concurrency"
                  type="number"
                  min="1"
                /><span class="cg-field-hint">Default: 2</span></label
              >
            </div>

            <!-- Rate limit -->
            <p class="cg-subsection-label">Rate limiting</p>
            <label class="cg-check cg-mb">
              <input type="checkbox" v-model="reg.rate_limit_enabled" /> Enable
              per-user rate limit
            </label>
            <template v-if="reg.rate_limit_enabled">
              <div class="cg-two-col">
                <label
                  >Requests per window<input
                    v-model.number="reg.rate_limit_rps"
                    type="number"
                    min="1"
                /></label>
                <label
                  >Window (s)<input
                    v-model.number="reg.rate_limit_window"
                    type="number"
                    min="1"
                /></label>
              </div>
              <label>
                Enforcement
                <select v-model="reg.rate_limit_enforcement">
                  <option value="block">block (429)</option>
                  <option value="warn">warn (header only)</option>
                </select>
              </label>
              <p class="cg-subsection-label">Per-group overrides</p>
              <span
                class="cg-field-hint"
                style="display: block; margin-bottom: 0.5rem"
                >Members of these groups get their own budget instead of the
                registry-wide one above.</span
              >
              <div
                v-for="g in reg.rate_limit_groups"
                :key="g.id"
                class="cg-condition-item"
              >
                <label
                  >Group name<input v-model="g.name" placeholder="oidc:ci"
                /></label>
                <div class="cg-two-col">
                  <label
                    >Requests per window<input
                      v-model.number="g.requests_per_window"
                      type="number"
                      min="1"
                  /></label>
                  <label
                    >Window (s)<input
                      v-model.number="g.window_secs"
                      type="number"
                      min="1"
                  /></label>
                </div>
                <label>
                  Enforcement
                  <select v-model="g.enforcement">
                    <option value="">— inherit —</option>
                    <option value="block">block (429)</option>
                    <option value="warn">warn (header only)</option>
                  </select>
                </label>
                <button
                  class="cg-btn-remove"
                  @click="removeRateLimitGroup(reg, g.id)"
                >
                  Remove group
                </button>
              </div>
              <button class="cg-btn-add" @click="addRateLimitGroup(reg)">
                + Add group override
              </button>
            </template>

            <!-- Quota (local/hybrid only) -->
            <template v-if="isLocalOrHybrid(reg)">
              <p class="cg-subsection-label">Publish quota</p>
              <label class="cg-check cg-mb">
                <input type="checkbox" v-model="reg.quota_enabled" /> Enable
                publish quota
              </label>
              <template v-if="reg.quota_enabled">
                <div class="cg-two-col">
                  <label
                    >Max bytes per user<input
                      v-model="reg.quota_max_bytes"
                      placeholder="e.g. 1073741824"
                  /></label>
                  <label
                    >Max packages per user<input
                      v-model="reg.quota_max_packages"
                      placeholder="e.g. 100"
                  /></label>
                </div>
                <label
                  >Warn threshold (%)<input
                    v-model.number="reg.quota_warn_threshold_pct"
                    type="number"
                    min="1"
                    max="100"
                  /><span class="cg-field-hint"
                    >Percentage of the quota at which a warning header is
                    returned (default: 80).</span
                  ></label
                >
                <label>
                  Enforcement
                  <select v-model="reg.quota_enforcement">
                    <option value="block">block (429)</option>
                    <option value="warn">warn (header only)</option>
                  </select>
                </label>
              </template>
            </template>

            <!-- Beta channel (local/hybrid only) -->
            <template v-if="isLocalOrHybrid(reg)">
              <p class="cg-subsection-label">Beta channel</p>
              <label class="cg-check cg-mb">
                <input type="checkbox" v-model="reg.beta_channel_enabled" />
                Gate pre-release versions to beta members
              </label>
            </template>

            <!-- Versioning (local/hybrid only) -->
            <template v-if="isLocalOrHybrid(reg)">
              <p class="cg-subsection-label">Versioning policy</p>
              <label class="cg-check cg-mb">
                <input type="checkbox" v-model="reg.versioning_enabled" />
                Enforce versioning rules at publish time
              </label>
              <template v-if="reg.versioning_enabled">
                <label class="cg-check">
                  <input type="checkbox" v-model="reg.versioning_enforce_semver" />
                  Require valid semver (reject non-semver versions)
                </label>
                <label class="cg-check cg-mb">
                  <input type="checkbox" v-model="reg.versioning_allow_prerelease" />
                  Allow pre-release versions (e.g. <code>1.0.0-beta.1</code>)
                </label>
                <label
                  >Version regex (optional)<input
                    v-model="reg.versioning_pattern"
                    placeholder="^\d+\.\d+\.\d+$"
                  /><span class="cg-field-hint"
                    >Reject publishes where the version string doesn't match this
                    pattern.</span
                  ></label
                >
              </template>
            </template>

            <!-- Signing (local/hybrid only) -->
            <template v-if="isLocalOrHybrid(reg)">
              <p class="cg-subsection-label">Artifact signing</p>
              <label class="cg-check cg-mb">
                <input type="checkbox" v-model="reg.signing_enabled" />
                Accept artifact signatures at publish time
              </label>
              <template v-if="reg.signing_enabled">
                <label class="cg-check cg-mb">
                  <input type="checkbox" v-model="reg.signing_required" />
                  Require signature (reject publishes without
                  <code>X-Artifact-Signature</code>)
                </label>
                <label
                  >Allowed signature types (comma-separated, optional)<input
                    v-model="reg.signing_allowed_types"
                    placeholder="pgp, ed25519"
                  /><span class="cg-field-hint"
                    >Leave blank to accept any type.</span
                  ></label
                >
                <label class="cg-check cg-mb">
                  <input
                    type="checkbox"
                    v-model="reg.signing_verify_on_download"
                  />
                  Verify stored signatures on every download
                </label>
                <label v-if="reg.signing_verify_on_download"
                  >Trusted Ed25519 public keys (comma-separated hex)<input
                    v-model="reg.signing_trusted_keys"
                    placeholder="3b6a27bcceb6a42d62a3a8d02a6f0d73…"
                  /><span class="cg-field-hint"
                    >Verification fails closed: a stored signature that matches
                    no key here — or whose type cannot be verified — fails the
                    download with <code>502</code>. Only unsigned artifacts are
                    exempt.</span
                  ></label
                >
              </template>
            </template>

            <!-- Repository metadata signing (deb/rpm) -->
            <template v-if="reg.type === 'deb' || reg.type === 'rpm'">
              <p class="cg-subsection-label">Repository metadata signing</p>
              <label class="cg-check cg-mb">
                <input type="checkbox" v-model="reg.repo_signing_enabled" />
                Sign the generated repository metadata
              </label>
              <template v-if="reg.repo_signing_enabled">
                <label
                  >Ed25519 seed (64 hex chars)<input
                    v-model="reg.repo_signing_seed_hex"
                    placeholder="0123456789abcdef…"
                  /><span class="cg-field-hint"
                    >Keep this stable — it is the identity APT/DNF clients pin
                    to. Treat it as a secret.</span
                  ></label
                >
                <div class="cg-two-col">
                  <label
                    >User ID (optional)<input
                      v-model="reg.repo_signing_user_id"
                      placeholder="BatleHub Repo &lt;repo@example.com&gt;"
                  /></label>
                  <label
                    >Created (unix seconds, optional)<input
                      v-model="reg.repo_signing_created"
                      placeholder="1700000000"
                    /><span class="cg-field-hint"
                      >Part of the key fingerprint, so it must not change.</span
                    ></label
                  >
                </div>
              </template>
            </template>

            <!-- SBOM -->
            <p class="cg-subsection-label">SBOM</p>
            <label class="cg-check cg-mb">
              <input type="checkbox" v-model="reg.sbom_enabled" />
              Generate an SBOM for cached and published artifacts
            </label>
            <template v-if="reg.sbom_enabled">
              <label
                >Formats (comma-separated)<input
                  v-model="reg.sbom_formats"
                  placeholder="spdx, cyclonedx"
              /></label>
              <label class="cg-check">
                <input type="checkbox" v-model="reg.sbom_required" />
                Reject publishes with no discoverable dependency manifest
              </label>
              <label class="cg-check cg-mb">
                <input type="checkbox" v-model="reg.sbom_fetch_upstream" />
                Prefer a pre-built SBOM from the upstream when one exists
              </label>
            </template>

            <!-- Integrity -->
            <p class="cg-subsection-label">Artifact integrity</p>
            <label class="cg-check cg-mb">
              <input type="checkbox" v-model="reg.integrity_customised" />
              Customise checksum verification
            </label>
            <span
              v-if="!reg.integrity_customised"
              class="cg-field-hint"
              style="display: block; margin-bottom: 0.5rem"
              >Defaults apply: verify against any advertised checksum and block
              on a mismatch; warn (never block) when the upstream advertises
              none.</span
            >
            <template v-if="reg.integrity_customised">
              <label class="cg-check">
                <input type="checkbox" v-model="reg.integrity_enabled" />
                Verify advertised checksums
              </label>
              <label class="cg-check">
                <input
                  type="checkbox"
                  v-model="reg.integrity_block_on_mismatch"
                />
                Fail the download on a mismatch
              </label>
              <label class="cg-check">
                <input
                  type="checkbox"
                  v-model="reg.integrity_require_metadata"
                />
                Block downloads with no advertised checksum
              </label>
              <label class="cg-check cg-mb">
                <input
                  type="checkbox"
                  v-model="reg.integrity_verify_on_serve"
                />
                Re-hash cached bytes on every serve
              </label>
              <span
                v-if="reg.integrity_verify_on_serve"
                class="cg-field-hint"
                style="display: block; margin-bottom: 0.5rem"
                >Catches storage corruption or tampering after caching, at the
                cost of hashing the bytes on each serve.</span
              >
              <label v-if="reg.integrity_require_metadata"
                >Bypass roles (comma-separated, optional)<input
                  v-model="reg.integrity_bypass_roles"
                  placeholder="admin"
                /><span class="cg-field-hint"
                  >Roles exempt from the missing-checksum gate. A mismatch is
                  never bypassable.</span
                ></label
              >
            </template>

            <!-- Rules -->
            <p class="cg-subsection-label">Rules</p>
            <label class="cg-check">
              <input type="checkbox" v-model="reg.rule_age_gate_enabled" />
              Release age gate
            </label>
            <template v-if="reg.rule_age_gate_enabled">
              <label
                >Min age (s)<input
                  v-model.number="reg.rule_age_gate_min_age"
                  type="number"
                  min="0"
                /><span class="cg-field-hint"
                  >Reject downloads of packages younger than this many
                  seconds.</span
                ></label
              >
              <label class="cg-check cg-mb">
                <input
                  type="checkbox"
                  v-model="reg.rule_age_gate_deny_missing_timestamp"
                />
                Deny packages whose upstream publishes no timestamp
              </label>
              <span
                v-if="reg.rule_age_gate_deny_missing_timestamp"
                class="cg-field-hint"
                style="display: block; margin-bottom: 0.5rem"
                >Otherwise the check is skipped for those packages and the
                download is allowed. Worth enabling on registries where the
                field is optional (e.g. conda).</span
              >
              <label
                >Bypass roles (comma-separated, optional)<input
                  v-model="reg.rule_age_gate_bypass_roles"
                  type="text"
                  placeholder="admin"
                /><span class="cg-field-hint"
                  >Roles that can bypass the gate. Leave blank for
                  none.</span
                ></label
              >
            </template>
            <label class="cg-check">
              <input type="checkbox" v-model="reg.rule_deny_latest_enabled" />
              Deny <code>@latest</code> / unpinned version requests
            </label>
            <template v-if="reg.rule_deny_latest_enabled">
              <label
                >Bypass roles (comma-separated, optional)<input
                  v-model="reg.rule_deny_latest_bypass_roles"
                  type="text"
                  placeholder="admin"
                /><span class="cg-field-hint"
                  >Roles that can bypass the gate. Leave blank for
                  none.</span
                ></label
              >
            </template>
            <label class="cg-check">
              <input type="checkbox" v-model="reg.rule_signed_release_enabled" />
              Require a signed release
            </label>
            <template v-if="reg.rule_signed_release_enabled">
              <span class="cg-field-hint" style="display: block; margin-bottom: 0.5rem"
                >Gates on the upstream's best-effort signature signal (a
                <code>.asc</code>/<code>.sig</code> asset, an extension
                signature blob) — <strong>not</strong> cryptographic
                verification. Use <em>Artifact signing</em> above for that.</span
              >
              <label class="cg-check cg-mb">
                <input
                  type="checkbox"
                  v-model="reg.rule_signed_release_deny_missing"
                />
                Deny registries that report no signature signal at all
              </label>
              <span
                v-if="reg.rule_signed_release_deny_missing"
                class="cg-field-hint"
                style="display: block; margin-bottom: 0.5rem"
                >npm, PyPI, crates.io and Maven report no signal, so this denies
                them outright rather than skipping the check.</span
              >
              <label
                >Bypass roles (comma-separated, optional)<input
                  v-model="reg.rule_signed_release_bypass_roles"
                  type="text"
                  placeholder="admin"
              /></label>
            </template>
            <label class="cg-check">
              <input type="checkbox" v-model="reg.rule_license_gate_enabled" />
              Licence gate
            </label>
            <template v-if="reg.rule_license_gate_enabled">
              <span class="cg-field-hint" style="display: block; margin-bottom: 0.5rem"
                >The licence is read from the archive, so it is unknown until
                the artifact has been fetched once — the first request for an
                uncached package is governed by
                <em>Allow unknown licences</em>, not by the lists. Matching is
                case-insensitive but literal:
                <code>MIT</code> does not match <code>MIT OR Apache-2.0</code>.</span
              >
              <label
                >Allowed licences (comma-separated, optional)<input
                  v-model="reg.rule_license_gate_allow"
                  placeholder="MIT, Apache-2.0, BSD-3-Clause"
                /><span class="cg-field-hint"
                  >When set, a declared licence matching none of these is
                  refused.</span
                ></label
              >
              <label
                >Denied licences (comma-separated, optional)<input
                  v-model="reg.rule_license_gate_deny"
                  placeholder="AGPL-3.0, SSPL-1.0"
                /><span class="cg-field-hint"
                  >Checked first, so a deny entry always wins.</span
                ></label
              >
              <label class="cg-check">
                <input
                  type="checkbox"
                  v-model="reg.rule_license_gate_allow_unknown"
                />
                Allow unknown licences
              </label>
              <label class="cg-check cg-mb">
                <input type="checkbox" v-model="reg.rule_license_gate_block" />
                Block downloads (otherwise warn-only, surfaced in the UI)
              </label>
              <p v-if="licenseGateDeniesEverything(reg)" class="cg-hint">
                <strong>This combination denies every download.</strong>
                <code>{{ reg.type }}</code> has no manifest parser, so the
                licence is always unknown — and unknown licences are being
                blocked. Licence extraction covers cargo, maven, npm, nuget and
                pypi. Allow unknown licences, or drop the rule here.
              </p>
              <label
                >Bypass roles (comma-separated, optional)<input
                  v-model="reg.rule_license_gate_bypass_roles"
                  type="text"
                  placeholder="admin"
              /></label>
            </template>
            <label class="cg-check">
              <input type="checkbox" v-model="reg.rule_version_gate_enabled" />
              Version gate
            </label>
            <template v-if="reg.rule_version_gate_enabled">
              <label
                >Allowed versions (one per line, optional)
                <textarea v-model="reg.rule_version_gate_allow" rows="2" />
                <span class="cg-field-hint"
                  >Exact versions or semver ranges
                  (<code>&gt;=1.2.0, &lt;2.0.0</code>). When set, anything
                  matching none of them is rejected. One per line because a
                  range contains commas.</span
                >
              </label>
              <label
                >Blocked versions (one per line, optional)
                <textarea v-model="reg.rule_version_gate_block" rows="2" />
                <span class="cg-field-hint"
                  >Specific versions or ranges with known issues.</span
                >
              </label>
              <label
                >Bypass roles (comma-separated, optional)<input
                  v-model="reg.rule_version_gate_bypass_roles"
                  type="text"
                  placeholder="admin"
              /></label>
              <span
                v-if="
                  !reg.rule_version_gate_allow.trim() &&
                  !reg.rule_version_gate_block.trim()
                "
                class="cg-field-hint"
                style="display: block; margin-bottom: 0.5rem"
                >Both lists are empty, so the rule is left out of the config —
                it would gate nothing.</span
              >
            </template>
            <label class="cg-check">
              <input type="checkbox" v-model="reg.rule_cve_gate_enabled" />
              CVE gate (uses <code>[vulnerability_scan]</code> findings)
            </label>
            <template v-if="reg.rule_cve_gate_enabled">
              <label
                >Minimum severity<select v-model="reg.rule_cve_gate_min_severity">
                  <option value="unknown">Unknown</option>
                  <option value="low">Low</option>
                  <option value="medium">Medium</option>
                  <option value="high">High</option>
                  <option value="critical">Critical</option>
                </select></label
              >
              <label class="cg-check cg-mb">
                <input type="checkbox" v-model="reg.rule_cve_gate_block" />
                Block downloads (otherwise warn-only, surfaced in the UI)
              </label>
              <label
                >Bypass roles (comma-separated, optional)<input
                  v-model="reg.rule_cve_gate_bypass_roles"
                  type="text"
                  placeholder="admin"
                /><span class="cg-field-hint"
                  >Roles that can bypass the gate even when blocking.</span
                ></label
              >
            </template>
            <label class="cg-check">
              <input
                type="checkbox"
                v-model="reg.rule_trusted_publisher_enabled"
              />
              Trusted publisher allowlist
            </label>
            <template v-if="reg.rule_trusted_publisher_enabled">
              <label
                >Allowed publishers<input
                  v-model="reg.rule_trusted_publisher_allow"
                  type="text"
                  placeholder="my-org, trusted-user"
                /><span class="cg-field-hint"
                  >Comma-separated org/user/scope names. Supported for
                  GitHub, GitLab, Forgejo, npm, OpenVSX, and VS Code
                  Marketplace; not yet for Cargo — an unsupported registry
                  denies every request.</span
                ></label
              >
              <label
                >Bypass roles (comma-separated, optional)<input
                  v-model="reg.rule_trusted_publisher_bypass_roles"
                  type="text"
                  placeholder="admin"
                /><span class="cg-field-hint"
                  >Roles that can bypass the gate.</span
                ></label
              >
            </template>

            <!-- Feature flags -->
            <p class="cg-subsection-label">Feature flags</p>
            <label class="cg-check cg-mb">
              <input type="checkbox" v-model="reg.feature_flags_socket_badge" />
              Show socket.dev supply-chain badge per version
            </label>

            <!-- Upstream auth -->
            <p class="cg-subsection-label">Upstream authentication</p>
            <label>
              Auth type
              <select v-model="reg.upstream_auth_type">
                <option value="">None</option>
                <option value="bearer">Bearer token</option>
                <option value="basic">Basic (username + password)</option>
                <option value="header">Custom header</option>
              </select>
            </label>
            <template v-if="reg.upstream_auth_type === 'bearer'">
              <label
                >Token<input
                  v-model="reg.upstream_auth_token"
                  placeholder="ghp_..."
              /></label>
            </template>
            <template v-else-if="reg.upstream_auth_type === 'basic'">
              <div class="cg-two-col">
                <label
                  >Username<input v-model="reg.upstream_auth_username"
                /></label>
                <label
                  >Password<input
                    v-model="reg.upstream_auth_password"
                    type="password"
                /></label>
              </div>
            </template>
            <template v-else-if="reg.upstream_auth_type === 'header'">
              <div class="cg-two-col">
                <label
                  >Header name<input
                    v-model="reg.upstream_auth_header_name"
                    placeholder="X-API-Key"
                /></label>
                <label
                  >Header value<input v-model="reg.upstream_auth_header_value"
                /></label>
              </div>
            </template>

            <!-- TLS -->
            <p class="cg-subsection-label">Upstream TLS</p>
            <label
              >Custom CA certificate path (optional)<input
                v-model="reg.tls_ca_cert_path"
                placeholder="/etc/ssl/corp-ca.pem"
              /><span class="cg-field-hint"
                >PEM-encoded CA to trust for this registry's upstream. Only
                needed for self-signed certificates.</span
              ></label
            >

            <!-- Per-registry egress proxy -->
            <p class="cg-subsection-label">Egress proxy</p>
            <label class="cg-check cg-mb">
              <input type="checkbox" v-model="reg.proxy_enabled" />
              Use a different proxy from the global one
            </label>
            <template v-if="reg.proxy_enabled">
              <label
                >Proxy URL<input
                  v-model="reg.proxy_url"
                  placeholder="http://proxy.corp:3128"
              /></label>
              <div class="cg-two-col">
                <label
                  >Username (optional)<input v-model="reg.proxy_username"
                /></label>
                <label
                  >Password (optional)<input
                    v-model="reg.proxy_password"
                    type="password"
                /></label>
              </div>
              <label
                >No-proxy list (optional)<input
                  v-model="reg.proxy_no_proxy"
                  placeholder="localhost,127.0.0.1,.internal"
                /><span class="cg-field-hint"
                  >Comma-separated hosts or suffixes that bypass the
                  proxy.</span
                ></label
              >
            </template>
          </div>

        </div>
        <button class="cg-btn-add" @click="addRegistry">+ Add registry</button>
      </section>

      <!-- IP Blocking -->
      <section class="cg-section">
        <h3>IP Blocking</h3>
        <label class="cg-check cg-mb">
          <input type="checkbox" v-model="ipBlocking.enabled" /> Enable
          fail2ban-style IP blocking
        </label>
        <template v-if="ipBlocking.enabled">
          <div class="cg-two-col">
            <label
              >Violation threshold<input
                v-model.number="ipBlocking.violation_threshold"
                type="number"
                min="1"
              /><span class="cg-field-hint"
                >Violations before auto-block</span
              ></label
            >
            <label
              >Window (s)<input
                v-model.number="ipBlocking.violation_window_secs"
                type="number"
                min="1"
            /></label>
          </div>
          <label
            >Ban duration (s)<input
              v-model.number="ipBlocking.ban_duration_secs"
              type="number"
              min="1"
          /></label>
          <label
            >Trigger on status codes<input
              v-model="ipBlocking.trigger_on_status"
              placeholder="429, 401"
            /><span class="cg-field-hint"
              >Comma-separated HTTP status codes that count as violations.</span
            ></label
          >
          <p class="cg-hint">
            Trusted proxies are configured once for the whole server, under
            <strong>Server</strong> above — this section's own
            <code>trusted_proxies</code> key is deprecated.
          </p>
        </template>
      </section>

      <!-- Vulnerability scan -->
      <section class="cg-section">
        <h3>Vulnerability scan</h3>
        <label class="cg-check cg-mb">
          <input type="checkbox" v-model="vulnerabilityScan.enabled" /> Periodically
          re-check cached SBOMs against the OSV database
        </label>
        <template v-if="vulnerabilityScan.enabled">
          <label
            >Interval (s)<input
              v-model.number="vulnerabilityScan.interval_secs"
              type="number"
              min="60"
            /><span class="cg-field-hint"
              >How often to re-scan. Default 86400 (daily).</span
            ></label
          >
          <label
            >OSV API URL (optional)<input
              v-model="vulnerabilityScan.osv_api_url"
              placeholder="https://api.osv.dev"
            /><span class="cg-field-hint"
              >Leave blank to use the public OSV API.</span
            ></label
          >
          <label
            >Batch size<input
              v-model.number="vulnerabilityScan.batch_size"
              type="number"
              min="1"
            /><span class="cg-field-hint"
              >SBOMs processed per page. Default 100.</span
            ></label
          >
        </template>
      </section>

      <!-- Statistics -->
      <section class="cg-section">
        <h3>Statistics</h3>
        <label class="cg-check">
          <input type="checkbox" v-model="stats.metrics_enabled" />
          Expose Prometheus metrics on <code>/metrics</code>
        </label>
        <label class="cg-check cg-mb">
          <input type="checkbox" v-model="stats.history_enabled" />
          Record request history for the dashboard trends
        </label>
        <label v-if="stats.history_enabled"
          >History retention (days)<input
            v-model.number="stats.history_retention_days"
            type="number"
            min="0"
          /><span class="cg-field-hint"
            >Rows older than this are pruned. Default 30.
            <code>0</code> disables pruning rather than disabling history —
            untick the box above for that.</span
          ></label
        >
        <span
          v-else
          class="cg-field-hint"
          style="display: block"
          >The dashboard's trend charts stay empty; live counters still
          work.</span
        >
      </section>

      <!-- Subdomain routing -->
      <section class="cg-section">
        <h3>Subdomain routing</h3>
        <label class="cg-check cg-mb">
          <input type="checkbox" v-model="subdomainRouting.enabled" />
          Derive a host for every registry from its name
        </label>
        <template v-if="subdomainRouting.enabled">
          <label
            >Base domain<input
              v-model="subdomainRouting.base_domain"
              placeholder="hub.example.com"
            /><span class="cg-field-hint"
              >A registry named <code>npm1</code> is then served at
              <code>npm1.hub.example.com</code>. Required — the server refuses to
              start with wildcard routing on and no base domain. Registry names
              must be valid DNS labels.</span
            ></label
          >
          <label>
            Advertised scheme
            <select v-model="subdomainRouting.scheme">
              <option value="https">https</option>
              <option value="http">http</option>
            </select>
            <span class="cg-field-hint"
              >Only used when rendering public URLs in the API and UI; a
              request's own scheme decides routing.</span
            >
          </label>
        </template>
      </section>

      <!-- Egress proxy -->
      <section class="cg-section">
        <h3>Egress proxy</h3>
        <label class="cg-check cg-mb">
          <input type="checkbox" v-model="upstreamProxy.enabled" />
          Route upstream fetches through an HTTP proxy
        </label>
        <template v-if="upstreamProxy.enabled">
          <label
            >Proxy URL<input
              v-model="upstreamProxy.url"
              placeholder="http://proxy.corp:3128"
            /><span class="cg-field-hint"
              >Applies to every registry that does not override it in its
              advanced options.</span
            ></label
          >
          <div class="cg-two-col">
            <label
              >Username (optional)<input v-model="upstreamProxy.username"
            /></label>
            <label
              >Password (optional)<input
                v-model="upstreamProxy.password"
                type="password"
            /></label>
          </div>
          <label
            >No-proxy list (optional)<input
              v-model="upstreamProxy.no_proxy"
              placeholder="localhost,127.0.0.1,.internal"
            /><span class="cg-field-hint"
              >Comma-separated hosts or suffixes that bypass the proxy.</span
            ></label
          >
        </template>
      </section>

      <!-- Notifications -->
      <section class="cg-section">
        <h3>Notifications</h3>
        <label class="cg-check cg-mb">
          <input type="checkbox" v-model="notifications.enabled" />
          Send notifications on registry events
        </label>
        <template v-if="notifications.enabled">
          <p class="cg-subsection-label">Outbound channels</p>
          <div
            v-for="ch in notifications.channels"
            :key="ch.id"
            class="cg-list-item"
          >
            <div class="cg-two-col">
              <label
                >Name<input v-model="ch.name" placeholder="ops-slack"
              /></label>
              <label>
                Type
                <select v-model="ch.type">
                  <option value="slack">Slack</option>
                  <option value="teams">Microsoft Teams</option>
                  <option value="webhook">Generic webhook</option>
                  <option value="email">Email (SMTP)</option>
                </select>
              </label>
            </div>
            <template v-if="ch.type === 'email'">
              <div class="cg-two-col">
                <label
                  >SMTP host<input v-model="ch.smtp_host" placeholder="smtp.example.com"
                /></label>
                <label
                  >SMTP port<input
                    v-model.number="ch.smtp_port"
                    type="number"
                    min="1"
                    max="65535"
                /></label>
              </div>
              <div class="cg-two-col">
                <label
                  >SMTP user (optional)<input v-model="ch.smtp_user"
                /></label>
                <label
                  >SMTP password (optional)<input
                    v-model="ch.smtp_password"
                    type="password"
                /></label>
              </div>
              <label
                >From<input v-model="ch.from" placeholder="batlehub@example.com"
              /></label>
              <label
                >To (comma-separated)<input
                  v-model="ch.to"
                  placeholder="ops@example.com"
              /></label>
              <label class="cg-check cg-mb">
                <input type="checkbox" v-model="ch.tls" /> Use STARTTLS
              </label>
            </template>
            <template v-else>
              <label
                >Webhook URL<input
                  v-model="ch.url"
                  placeholder="https://hooks.slack.com/services/…"
              /></label>
              <label v-if="ch.type === 'webhook'"
                >HMAC secret (optional)<input
                  v-model="ch.secret"
                  type="password"
                /><span class="cg-field-hint"
                  >When set, each POST carries an
                  <code>X-BatleHub-Signature-256</code> header so the receiver
                  can verify it.</span
                ></label
              >
            </template>
            <label
              >Timeout (s)<input
                v-model.number="ch.timeout_secs"
                type="number"
                min="1"
              /><span class="cg-field-hint">Default: 10</span></label
            >
            <button class="cg-btn-remove" @click="removeChannel(ch.id)">
              Remove channel
            </button>
          </div>
          <button class="cg-btn-add" @click="addChannel">+ Add channel</button>

          <p class="cg-subsection-label" style="margin-top: 1rem">
            Inbound webhooks
          </p>
          <span class="cg-field-hint" style="display: block; margin-bottom: 0.5rem"
            >Endpoints external systems can POST events to, at
            <code>/api/v1/webhooks/inbound/&lt;name&gt;</code>.</span
          >
          <div
            v-for="hook in notifications.inbound"
            :key="hook.id"
            class="cg-condition-item"
          >
            <div class="cg-two-col">
              <label
                >Name<input v-model="hook.name" placeholder="ci-scanner"
              /></label>
              <label
                >HMAC secret (optional)<input
                  v-model="hook.secret"
                  type="password"
                /><span class="cg-field-hint"
                  >Verifies <code>X-Hub-Signature-256</code>. Without it any
                  payload is accepted.</span
                ></label
              >
            </div>
            <button class="cg-btn-remove" @click="removeInboundHook(hook.id)">
              Remove webhook
            </button>
          </div>
          <button class="cg-btn-add" @click="addInboundHook">
            + Add inbound webhook
          </button>
        </template>
      </section>

      <!-- OpenTelemetry -->
      <section class="cg-section">
        <h3>OpenTelemetry</h3>
        <label class="cg-check cg-mb">
          <input type="checkbox" v-model="otel.enabled" /> Enable tracing
        </label>
        <template v-if="otel.enabled">
          <label
            >OTLP gRPC endpoint<input
              v-model="otel.endpoint"
              placeholder="http://localhost:4317"
          /></label>
          <label
            >Service name<input
              v-model="otel.service_name"
              placeholder="batlehub"
          /></label>
        </template>
      </section>
    </div>

    <!-- ── RIGHT: live preview ─────────────────────────────────────────── -->
    <div class="cg-preview">
      <div class="cg-preview-header">
        <span class="cg-filename">config.toml</span>
        <div class="cg-actions">
          <button class="cg-btn-action" @click="copyToml">
            {{ copied ? "Copied!" : "Copy" }}
          </button>
          <button class="cg-btn-action" @click="downloadToml">Download</button>
        </div>
      </div>
      <pre class="cg-code"><code v-html="highlightedToml" /></pre>
    </div>
  </div>
</template>

<style scoped>
/* One column is the floor, not the small-screen special case. The side-by-side
   arrangement is what gets asked for, in the `@container` block at the bottom,
   and only once there is room for it — the previous rules asked the *window*
   whether there was room, which is a different question on a page carrying a
   272px sidebar, and was the reason the form had become unusable at 1366 and
   1440.

   Flex-wrap rather than a grid, and the container declared here rather than on
   a wrapper, so that the two are the same element: an element cannot size its
   own grid columns from a query against itself, but its children can read it,
   and `flex-basis` on the children is the same layout expressed where the
   query is legal. `inline-size` and not `size` — the height here is the
   content's, and a container that must resolve its own height first cannot
   have one. */
.cg-root {
  container: cg / inline-size;
  display: flex;
  flex-wrap: wrap;
  gap: 2rem;
  align-items: flex-start;
  margin-top: 1.5rem;
  width: 100%;
}

/* ── Form column ────────────────────────────────────────────────────── */
.cg-form {
  flex: 1 1 100%;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.cg-section {
  border: 1px solid var(--vp-c-divider);
  border-radius: var(--radius);
  padding: 1rem 1.2rem;
}

/* Pixel Small — DESIGN.md spends it on exactly this: "a panel heading". The
   tracking is the Tracking Ladder's 0.04em step for a Silkscreen label rather
   than the 0.05em this was carrying, which was not a step at all. */
.cg-section h3 {
  margin: 0 0 0.75rem;
  font-family: var(--face-display);
  font-size: var(--t-px-sm);
  font-weight: 700;
  color: var(--ink);
  text-transform: uppercase;
  letter-spacing: 0.04em;
  border-bottom: 1px solid var(--vp-c-divider);
  padding-bottom: 0.4rem;
}

label {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  font-size: var(--t-body);
  color: var(--vp-c-text-2);
  margin-bottom: 0.45rem;
}

input[type="text"],
input[type="number"],
input[type="password"],
input:not([type]),
select,
textarea {
  padding: 0.35rem 0.6rem;
  border: 1px solid var(--vp-c-divider);
  border-radius: var(--radius);
  background: var(--vp-c-bg);
  color: var(--vp-c-text-1);
  font-size: var(--t-body);
  font-family: var(--vp-font-family-mono);
  width: 100%;
  box-sizing: border-box;
  transition: border-color 0.15s;
}

input:focus,
select:focus,
textarea:focus {
  outline: none;
  border-color: var(--vp-c-brand-1);
}

textarea {
  resize: vertical;
}

/* ── Radios + checkboxes ──────────────────────────────────────────── */
.cg-radio-row {
  display: flex;
  gap: 1.2rem;
  flex-wrap: wrap;
}

.cg-radio,
.cg-check {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 0.35rem;
  font-size: var(--t-body);
  color: var(--vp-c-text-1);
  cursor: pointer;
  margin-bottom: 0;
}

.cg-mb {
  margin-bottom: 0.6rem;
}

.cg-radio.cg-disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.cg-hint {
  font-size: var(--t-meta);
  color: var(--vp-c-text-2);
  margin: 0 0 0.6rem;
}

.cg-hint code {
  font-size: var(--t-meta);
}

/* ── Grid layouts ──────────────────────────────────────────────────
   `auto-fit` + a floor, rather than a fixed count unpicked by a breakpoint.
   These sit at four different nesting depths (a section, a list item, a
   condition row inside a list item) so the width available to them is never
   the width of anything a media query can name; what they can promise is that
   a field is 13rem or it is on its own line. `min(100%, …)` is what keeps that
   floor from becoming an overflow when the column really is narrower. */
.cg-two-col {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 13rem), 1fr));
  gap: 0.75rem;
}

.cg-three-col {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(min(100%, 10rem), 1fr));
  gap: 0.6rem;
}

/* ── List items ──────────────────────────────────────────────────── */
.cg-list-item {
  border: 1px solid var(--vp-c-divider);
  border-radius: var(--radius);
  padding: 0.75rem;
  margin-bottom: 0.75rem;
  background: var(--vp-c-bg-soft);
}

.cg-subitem {
  border: 1px solid var(--vp-c-divider);
  border-radius: var(--radius);
  padding: 0.6rem;
  margin-bottom: 0.5rem;
  background: var(--vp-c-bg);
}

.cg-condition-item {
  border: 1px dashed var(--vp-c-divider);
  border-radius: var(--radius);
  padding: 0.5rem;
  margin-bottom: 0.4rem;
  background: var(--vp-c-bg-soft);
}

/* ── Advanced panel ──────────────────────────────────────────────── */
.cg-advanced {
  border-top: 1px solid var(--vp-c-divider);
  margin-top: 0.75rem;
  padding-top: 0.75rem;
}

.cg-subsection-label {
  margin: 0.6rem 0 0.3rem;
  font-size: var(--t-meta);
  font-weight: 600;
  color: var(--vp-c-text-2);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

/* ── Labels / hints ──────────────────────────────────────────────── */
.cg-perm-label {
  margin: 0.4rem 0 0.25rem;
  font-size: var(--t-body);
  color: var(--vp-c-text-2);
  font-weight: 500;
}

.cg-field-hint {
  font-size: var(--t-meta);
  color: var(--vp-c-text-3);
  margin-top: 0.15rem;
}

.cg-field-hint code {
  font-size: var(--t-meta);
}

.cg-perm-hint {
  font-size: var(--t-meta);
  font-weight: 400;
  color: var(--vp-c-text-3);
}

/* ── Buttons ─────────────────────────────────────────────────────── */
.cg-btn-add {
  display: inline-block;
  padding: 0.3rem 0.8rem;
  font-size: var(--t-body);
  border: 1px dashed var(--vp-c-brand-2);
  border-radius: var(--radius);
  color: var(--vp-c-brand-1);
  background: transparent;
  cursor: pointer;
  transition: background 0.15s;
}
.cg-btn-add:hover {
  background: var(--vp-c-brand-soft);
}

.cg-btn-remove {
  font-size: var(--t-meta);
  padding: 0.2rem 0.6rem;
  margin-top: 0.3rem;
  border: 1px solid var(--vp-c-danger-1);
  border-radius: var(--radius);
  color: var(--vp-c-danger-1);
  background: transparent;
  cursor: pointer;
  transition: background 0.15s;
}
.cg-btn-remove:hover {
  background: color-mix(
    in srgb,
    var(--vp-c-danger-1) 10%,
    transparent
  );
}

.cg-btn-advanced {
  display: inline-block;
  margin: 0;
  padding: 0.2rem 0.6rem;
  font-size: var(--t-meta);
  border: 1px solid var(--vp-c-divider);
  border-radius: var(--radius);
  background: transparent;
  color: var(--vp-c-text-2);
  cursor: pointer;
  transition:
    background 0.15s,
    border-color 0.15s;
}
.cg-btn-advanced:hover {
  border-color: var(--vp-c-brand-1);
  color: var(--vp-c-brand-1);
  background: var(--vp-c-brand-soft);
}

/* ── Action rows (add/remove on same line) ───────────────────────── */
.cg-provider-actions,
.cg-registry-actions {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-top: 0.75rem;
}

/* ── Preview column ────────────────────────────────────────────────
   Stacked below the form until the container says otherwise, so it is sized
   here as a panel and re-sized as a sticky rail in the `@container` block. */
.cg-preview {
  flex: 1 1 100%;
  min-width: 0;
  height: 520px;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--vp-c-divider);
  border-radius: var(--radius);
  overflow: hidden;
}

.cg-preview-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.5rem 0.9rem;
  background: var(--vp-c-bg-soft);
  border-bottom: 1px solid var(--vp-c-divider);
  flex-shrink: 0;
}

.cg-filename {
  font-size: var(--t-body);
  font-family: var(--vp-font-family-mono);
  color: var(--vp-c-text-2);
}

.cg-actions {
  display: flex;
  gap: 0.5rem;
}

.cg-btn-action {
  font-size: var(--t-meta);
  padding: 0.25rem 0.7rem;
  border: 1px solid var(--vp-c-divider);
  border-radius: var(--radius);
  background: var(--vp-c-bg);
  color: var(--vp-c-text-1);
  cursor: pointer;
  transition:
    border-color 0.15s,
    background 0.15s;
}

.cg-btn-action:hover {
  border-color: var(--vp-c-brand-1);
  background: var(--vp-c-brand-soft);
  color: var(--vp-c-brand-1);
}

.cg-code {
  margin: 0;
  padding: 0.9rem;
  font-size: var(--t-meta);
  line-height: 1.55;
  font-family: var(--vp-font-family-mono);
  background: var(--vp-c-bg);
  color: var(--vp-c-text-1);
  overflow-y: auto;
  flex: 1 1 0;
  white-space: pre;
}

/* ── Registry-type hints ─────────────────────────────────────────── */
.cg-registry-hint {
  border: 1px solid var(--vp-c-brand-soft);
  border-left: 1px solid var(--vp-c-brand-1);
  border-radius: var(--radius);
  padding: 0.65rem 0.8rem;
  margin: 0.5rem 0;
  background: var(--vp-c-brand-soft);
}

.cg-hint-title {
  font-size: var(--t-meta);
  font-weight: 600;
  color: var(--vp-c-brand-1);
  margin: 0 0 0.35rem;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.cg-hint-text {
  font-size: var(--t-meta);
  color: var(--vp-c-text-2);
  margin: 0 0 0.25rem;
}

.cg-hint-text code {
  font-size: var(--t-meta);
}

.cg-hint-code {
  font-size: var(--t-meta);
  font-family: var(--vp-font-family-mono);
  background: var(--vp-c-bg);
  border: 1px solid var(--vp-c-divider);
  border-radius: var(--radius);
  padding: 0.5rem 0.65rem;
  margin: 0.25rem 0 0;
  white-space: pre;
  overflow-x: auto;
  color: var(--vp-c-text-1);
  line-height: 1.5;
}

/* ── Responsive ────────────────────────────────────────────────────
   Three steps, each asked of the container and each stated as what it adds.
   The numbers are the widths the arrangement needs, not the widths of any
   particular screen: whether a 1440px window clears 60rem here depends on the
   sidebar, and that is precisely what the component should not have to know.
   ────────────────────────────────────────────────────────────────────────── */

/* 60rem = 960px, which is what this page hands the component on a 1366px
   laptop — the width the old 1300px window breakpoint got wrong by exactly
   the sidebar. Below it the preview goes under the form rather than beside
   it; above it there is room for a form column that still fits two 13rem
   fields after the preview has taken its share. The preview is a proportion
   with both ends pinned: never so narrow that a TOML line has nowhere to go,
   never so wide that it is reading room taken from the form it previews. */
@container cg (min-width: 60rem) {
  .cg-form {
    flex: 1 1 0;
  }

  .cg-preview {
    flex: 0 0 clamp(22rem, 34%, 35rem);
    position: sticky;
    top: calc(var(--vp-nav-height) + 1rem);
    height: calc(100vh - var(--vp-nav-height) - 2rem);
    min-height: 480px;
  }
}

/* 90rem = 1440px, reached on a 1920px screen. Only here is the form column
   itself wide enough that one stack of sections leaves half the row empty and
   a text input is 450px for a port number. The old rule asked this of a 1600px
   *window*, which on this page meant splitting a 442px column in two: inputs
   came out 80px wide, and that is the state the page was reported in. */
@container cg (min-width: 90rem) {
  .cg-form {
    display: block;
    columns: 2;
    column-gap: 1rem;
  }

  .cg-section {
    break-inside: avoid;
    margin-bottom: 1rem;
  }
}

/* ── Argon2 hash status ───────────────────────────────────────────── */
.cg-label-note {
  font-size: var(--t-meta);
  font-weight: 400;
  color: var(--vp-c-text-2);
  margin-left: 0.25rem;
}

.cg-hash-status {
  margin-top: 0.3rem;
  margin-bottom: 0.25rem;
  font-size: var(--t-meta);
  line-height: 1.4;
}

.cg-hash-computing {
  color: var(--vp-c-text-2);
}

/* Ready is a fact and takes ink; "not hashed yet" is waiting, which is copper's
   whole job. Neither needs a `.dark` twin — the tokens flip with the rendition,
   which is what the two hard-coded greens and yellows were doing by hand. */
.cg-hash-ready {
  color: var(--ink);
}

.cg-hash-warn {
  color: var(--copper);
}

.cg-hash-status code {
  font-size: var(--t-meta);
  background: var(--vp-code-bg);
  padding: 0.1em 0.3em;
  border-radius: var(--radius);
}
</style>

<style>
/* TOML syntax tokens.
   ────────────────────────────────────────────────────────────────────────────
   This is the site's one hand-rolled highlighter — every other code block goes
   through Shiki. It carried sixteen literal hex values, a GitHub-derived
   palette in two hand-maintained renditions, and one of the eight failed AA
   against this pane's own background (`--vp-c-bg` → `--ground`, paper):
   comment #6e7781 at 4.15:1. The other seven measured 4.60 to 6.93 and passed.
   `#6e7781` is the same value rejected when the Shiki theme was chosen for the
   rest of the site; it survived here because nothing had ever looked, and the
   rendered gate could not look — the default form state emits no comment, so
   that span never rendered on the scanned page (see docs/build/design-routes.mjs).

   Ratios are axe's, measured in the browser against the painted pixels, not
   computed from the token values. Calibrated first: axe returns 5.63:1 for
   --accent and 7.24:1 for --ink-dim, the two figures tokens.css asserts.

   Four colours now, not eight, because the palette has four and The One
   Synthetic Rule caps it. TOML has few enough token classes that the
   distinctions that matter survive: a section header is the accent, a key is
   ink, a value is copper, and the punctuation and comments recede into dim ink.
   Every ratio below is one `tokens.css` already asserts in both renditions, so
   there is nothing left here to measure separately — and no `.dark` twin to
   keep in step, since the tokens flip with the ground. */
.cg-hl-comment  { color: var(--ink-dim); font-style: italic; }
.cg-hl-bracket  { color: var(--ink-dim); }
.cg-hl-table    { color: var(--accent); font-weight: 600; }
.cg-hl-key      { color: var(--ink); }
.cg-hl-eq       { color: var(--ink-dim); }
.cg-hl-string   { color: var(--copper); }
.cg-hl-number   { color: var(--copper); }
.cg-hl-bool     { color: var(--accent); }
</style>
