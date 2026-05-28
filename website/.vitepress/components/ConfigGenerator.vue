<script setup lang="ts">
import { ref, computed } from 'vue'

// ── Types ───────────────────────────────────────────────────────────────────

type RegistryMode = 'proxy' | 'local' | 'hybrid'
type RegistryType = 'npm' | 'cargo' | 'openvsx' | 'vscode-marketplace' | 'goproxy' | 'github' | 'maven' | 'terraform' | 'rubygems' | 'composer'
type AuthRole = 'admin' | 'user' | 'anonymous'
type StorageBackendType = 'filesystem' | 's3'
type StorageMode = 'single' | 'multi'
type AuthType = 'token' | 'oidc' | 'kubernetes'
type UpstreamAuthType = '' | 'bearer' | 'basic' | 'header'
type Enforcement = 'block' | 'warn'

interface StorageBackend {
  id: number
  name: string
  type: StorageBackendType
  path: string
  bucket: string
  region: string
  endpoint_url: string
  force_path_style: boolean
  prefix: string
}

interface Token {
  id: number
  value: string
  role: AuthRole
  user_id: string
}

interface AuthProvider {
  id: number
  type: AuthType
  // token
  tokens: Token[]
  // oidc
  oidc_issuer: string
  oidc_client_id: string
  oidc_client_secret: string
  oidc_redirect_uri: string
  oidc_frontend_url: string
  oidc_user_id_claim: string
  oidc_role_claim: string
  // kubernetes
  k8s_api_server: string
  k8s_audiences: string
}

interface Registry {
  id: number
  name: string
  type: RegistryType
  mode: RegistryMode
  upstreams: string
  storage_backend: string
  rbac_anonymous: string
  rbac_user: string
  rbac_admin: string
  // advanced panel toggle
  showAdvanced: boolean
  // upstream auth
  upstream_auth_type: UpstreamAuthType
  upstream_auth_token: string
  upstream_auth_username: string
  upstream_auth_password: string
  upstream_auth_header_name: string
  upstream_auth_header_value: string
  // cache policy
  cache_metadata_ttl: number
  cache_artifact_ttl: string
  cache_idle_days: string
  cache_max_size_bytes: string
  cache_keep_latest_n: string
  // rate limit
  rate_limit_enabled: boolean
  rate_limit_rps: number
  rate_limit_window: number
  rate_limit_enforcement: Enforcement
  // quota (local/hybrid)
  quota_enabled: boolean
  quota_max_bytes: string
  quota_max_packages: string
  quota_enforcement: Enforcement
  // beta channel (local/hybrid)
  beta_channel_enabled: boolean
  // rules
  rule_age_gate_enabled: boolean
  rule_age_gate_min_age: number
  rule_deny_latest_enabled: boolean
}

// ── State ───────────────────────────────────────────────────────────────────

const server = ref({ host: '0.0.0.0', port: 8080, static_dir: '' })
const database = ref({ url: '', max_connections: 10 })

// Metadata cache backend
const metaCache = ref({ type: 'memory', url: '' })

// Upload limits
const limits = ref({ max_artifact_size_bytes: '' })

// IP blocking
const ipBlocking = ref({
  enabled: false,
  violation_threshold: 10,
  violation_window_secs: 300,
  ban_duration_secs: 3600,
  trigger_on_status: '429, 401',
})

// Storage
const storageMode = ref<StorageMode>('single')
const singleStorage = ref<{ type: StorageBackendType; path: string; bucket: string; region: string; endpoint_url: string; force_path_style: boolean; prefix: string }>({
  type: 'filesystem',
  path: './cache',
  bucket: '',
  region: 'us-east-1',
  endpoint_url: '',
  force_path_style: false,
  prefix: '',
})
let backendSeq = 0
const storageDefault = ref('primary')
const storageBackends = ref<StorageBackend[]>([
  { id: backendSeq++, name: 'primary', type: 'filesystem', path: './cache', bucket: '', region: 'us-east-1', endpoint_url: '', force_path_style: false, prefix: '' },
])

// OTel
const otel = ref({ enabled: false, endpoint: 'http://localhost:4317', service_name: 'batlehub' })

// Auth providers
let authSeq = 0
let tokenSeq = 0
const authProviders = ref<AuthProvider[]>([{
  id: authSeq++,
  type: 'token',
  tokens: [{ id: tokenSeq++, value: '', role: 'admin', user_id: 'admin' }],
  oidc_issuer: '',
  oidc_client_id: '',
  oidc_client_secret: '',
  oidc_redirect_uri: '',
  oidc_frontend_url: '',
  oidc_user_id_claim: 'sub',
  oidc_role_claim: 'role',
  k8s_api_server: '',
  k8s_audiences: 'batlehub',
}])

// Registries
let registrySeq = 0

const defaultUpstream: Record<RegistryType, string> = {
  npm: 'https://registry.npmjs.org',
  cargo: 'https://index.crates.io',
  openvsx: 'https://open-vsx.org',
  'vscode-marketplace': 'https://marketplace.visualstudio.com',
  goproxy: 'https://proxy.golang.org',
  github: 'https://api.github.com',
  maven: 'https://repo1.maven.org/maven2',
  terraform: 'https://registry.terraform.io',
  rubygems: 'https://rubygems.org',
  composer: 'https://repo.packagist.org',
}

function defaultRegistry(type: RegistryType = 'npm'): Registry {
  return {
    id: registrySeq++,
    name: type,
    type,
    mode: 'proxy',
    upstreams: defaultUpstream[type],
    storage_backend: '',
    rbac_anonymous: 'releases:read, source:read',
    rbac_user: 'releases:read, source:read',
    rbac_admin: '*',
    showAdvanced: false,
    upstream_auth_type: '',
    upstream_auth_token: '',
    upstream_auth_username: '',
    upstream_auth_password: '',
    upstream_auth_header_name: '',
    upstream_auth_header_value: '',
    cache_metadata_ttl: 300,
    cache_artifact_ttl: '',
    cache_idle_days: '',
    cache_max_size_bytes: '',
    cache_keep_latest_n: '',
    rate_limit_enabled: false,
    rate_limit_rps: 100,
    rate_limit_window: 60,
    rate_limit_enforcement: 'block',
    quota_enabled: false,
    quota_max_bytes: '',
    quota_max_packages: '',
    quota_enforcement: 'block',
    beta_channel_enabled: false,
    rule_age_gate_enabled: false,
    rule_age_gate_min_age: 3600,
    rule_deny_latest_enabled: false,
  }
}

const registries = ref<Registry[]>([defaultRegistry('npm')])

// ── Helpers ─────────────────────────────────────────────────────────────────

function q(s: string) {
  return `"${s.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`
}

function permsToToml(csv: string): string {
  const perms = csv.split(',').map(p => p.trim()).filter(Boolean)
  if (!perms.length) return '[]'
  return `[${perms.map(q).join(', ')}]`
}

function listToToml(csv: string): string {
  const items = csv.split(',').map(p => p.trim()).filter(Boolean)
  if (!items.length) return '[]'
  return `[${items.map(q).join(', ')}]`
}

function backendFields(b: { type: StorageBackendType; path: string; bucket: string; region: string; endpoint_url: string; force_path_style: boolean; prefix: string }): string[] {
  const lines: string[] = []
  lines.push(`type = ${q(b.type)}`)
  if (b.type === 'filesystem') {
    lines.push(`path = ${q(b.path || './cache')}`)
  } else {
    lines.push(`bucket = ${q(b.bucket)}`)
    lines.push(`region = ${q(b.region)}`)
    if (b.prefix) lines.push(`prefix = ${q(b.prefix)}`)
    if (b.endpoint_url) lines.push(`endpoint_url = ${q(b.endpoint_url)}`)
    if (b.force_path_style) lines.push(`force_path_style = true`)
  }
  return lines
}

// ── TOML generation ─────────────────────────────────────────────────────────

const toml = computed(() => {
  const lines: string[] = []

  // [server]
  lines.push('[server]')
  lines.push(`host = ${q(server.value.host)}`)
  lines.push(`port = ${server.value.port}`)
  if (server.value.static_dir) lines.push(`static_dir = ${q(server.value.static_dir)}`)

  // [database]
  lines.push('')
  lines.push('[database]')
  lines.push(`type = "postgresql"`)
  lines.push(`url = ${q(database.value.url || 'postgresql://batlehub:changeme@localhost:5432/batlehub')}`)
  if (database.value.max_connections !== 10) lines.push(`max_connections = ${database.value.max_connections}`)

  // [cache]
  if (metaCache.value.type !== 'memory' || metaCache.value.url) {
    lines.push('')
    lines.push('[cache]')
    lines.push(`type = ${q(metaCache.value.type)}`)
    if (metaCache.value.type === 'redis' && metaCache.value.url) {
      lines.push(`url = ${q(metaCache.value.url)}`)
    }
  }

  // [limits]
  if (limits.value.max_artifact_size_bytes) {
    lines.push('')
    lines.push('[limits]')
    lines.push(`max_artifact_size_bytes = ${limits.value.max_artifact_size_bytes}`)
  }

  // [[auth]]
  for (const auth of authProviders.value) {
    lines.push('')
    lines.push('[[auth]]')
    lines.push(`type = ${q(auth.type)}`)
    if (auth.type === 'token') {
      const valid = auth.tokens.filter(t => t.value.trim())
      for (const tok of valid) {
        lines.push('')
        lines.push('[[auth.tokens]]')
        lines.push(`value = ${q(tok.value)}`)
        lines.push(`role = ${q(tok.role)}`)
        if (tok.user_id) lines.push(`user_id = ${q(tok.user_id)}`)
      }
    } else if (auth.type === 'oidc') {
      if (auth.oidc_issuer) lines.push(`issuer_url = ${q(auth.oidc_issuer)}`)
      if (auth.oidc_client_id) lines.push(`client_id = ${q(auth.oidc_client_id)}`)
      if (auth.oidc_client_secret) lines.push(`client_secret = ${q(auth.oidc_client_secret)}`)
      if (auth.oidc_redirect_uri) lines.push(`redirect_uri = ${q(auth.oidc_redirect_uri)}`)
      if (auth.oidc_frontend_url) lines.push(`frontend_url = ${q(auth.oidc_frontend_url)}`)
      if (auth.oidc_user_id_claim && auth.oidc_user_id_claim !== 'sub') lines.push(`user_id_claim = ${q(auth.oidc_user_id_claim)}`)
      if (auth.oidc_role_claim && auth.oidc_role_claim !== 'role') lines.push(`role_claim = ${q(auth.oidc_role_claim)}`)
    } else if (auth.type === 'kubernetes') {
      if (auth.k8s_api_server) lines.push(`api_server = ${q(auth.k8s_api_server)}`)
      if (auth.k8s_audiences) {
        const auds = auth.k8s_audiences.split(',').map(a => a.trim()).filter(Boolean)
        lines.push(`audiences = [${auds.map(q).join(', ')}]`)
      }
    }
  }

  // [storage]
  lines.push('')
  if (storageMode.value === 'single') {
    lines.push('[storage]')
    for (const l of backendFields(singleStorage.value)) lines.push(l)
  } else {
    lines.push('[storage]')
    lines.push(`default = ${q(storageDefault.value)}`)
    for (const b of storageBackends.value) {
      if (!b.name) continue
      lines.push('')
      lines.push('[[storage.backends]]')
      lines.push(`name = ${q(b.name)}`)
      for (const l of backendFields(b)) lines.push(l)
    }
  }

  // [[registries]]
  for (const reg of registries.value) {
    if (!reg.name) continue
    lines.push('')
    lines.push('[[registries]]')
    lines.push(`type = ${q(reg.type)}`)
    lines.push(`name = ${q(reg.name)}`)
    if (reg.mode !== 'proxy') lines.push(`mode = ${q(reg.mode)}`)
    if (reg.mode !== 'local') {
      const ups = reg.upstreams.split('\n').map(u => u.trim()).filter(Boolean)
      if (ups.length) lines.push(`upstreams = [${ups.map(q).join(', ')}]`)
    }
    if (storageMode.value === 'multi' && reg.storage_backend) {
      lines.push(`storage = ${q(reg.storage_backend)}`)
    }

    // [registries.rbac]
    lines.push('')
    lines.push('[registries.rbac]')
    lines.push(`anonymous = ${permsToToml(reg.rbac_anonymous)}`)
    lines.push(`user = ${permsToToml(reg.rbac_user)}`)
    lines.push(`admin = ${permsToToml(reg.rbac_admin)}`)

    // [registries.cache] — only emit non-default values
    const nonDefaultCache =
      reg.cache_metadata_ttl !== 300 ||
      reg.cache_artifact_ttl ||
      reg.cache_idle_days ||
      reg.cache_max_size_bytes ||
      reg.cache_keep_latest_n
    if (nonDefaultCache) {
      lines.push('')
      lines.push('[registries.cache]')
      if (reg.cache_metadata_ttl !== 300) lines.push(`metadata_ttl_secs = ${reg.cache_metadata_ttl}`)
      if (reg.cache_artifact_ttl) lines.push(`artifact_ttl_secs = ${reg.cache_artifact_ttl}`)
      if (reg.cache_idle_days) lines.push(`idle_days = ${reg.cache_idle_days}`)
      if (reg.cache_max_size_bytes) lines.push(`max_size_bytes = ${reg.cache_max_size_bytes}`)
      if (reg.cache_keep_latest_n) lines.push(`keep_latest_n = ${reg.cache_keep_latest_n}`)
    }

    // [registries.rate_limit]
    if (reg.rate_limit_enabled) {
      lines.push('')
      lines.push('[registries.rate_limit]')
      lines.push(`requests_per_window = ${reg.rate_limit_rps}`)
      lines.push(`window_secs = ${reg.rate_limit_window}`)
      if (reg.rate_limit_enforcement !== 'block') lines.push(`enforcement = ${q(reg.rate_limit_enforcement)}`)
    }

    // [registries.quota]
    if (reg.quota_enabled && (reg.mode === 'local' || reg.mode === 'hybrid')) {
      lines.push('')
      lines.push('[registries.quota]')
      if (reg.quota_max_bytes) lines.push(`max_storage_bytes_per_user = ${reg.quota_max_bytes}`)
      if (reg.quota_max_packages) lines.push(`max_packages_per_user = ${reg.quota_max_packages}`)
      if (reg.quota_enforcement !== 'block') lines.push(`enforcement = ${q(reg.quota_enforcement)}`)
    }

    // [registries.beta_channel]
    if (reg.beta_channel_enabled && (reg.mode === 'local' || reg.mode === 'hybrid')) {
      lines.push('')
      lines.push('[registries.beta_channel]')
      lines.push(`enabled = true`)
    }

    // [[registries.rules]]
    if (reg.rule_age_gate_enabled) {
      lines.push('')
      lines.push('[[registries.rules]]')
      lines.push(`kind = "release_age_gate"`)
      lines.push(`min_age_secs = ${reg.rule_age_gate_min_age}`)
    }
    if (reg.rule_deny_latest_enabled) {
      lines.push('')
      lines.push('[[registries.rules]]')
      lines.push(`kind = "deny_latest"`)
    }

    // [registries.upstream_auth]
    if (reg.upstream_auth_type) {
      lines.push('')
      lines.push('[registries.upstream_auth]')
      lines.push(`type = ${q(reg.upstream_auth_type)}`)
      if (reg.upstream_auth_type === 'bearer' && reg.upstream_auth_token) {
        lines.push(`token = ${q(reg.upstream_auth_token)}`)
      } else if (reg.upstream_auth_type === 'basic') {
        if (reg.upstream_auth_username) lines.push(`username = ${q(reg.upstream_auth_username)}`)
        if (reg.upstream_auth_password) lines.push(`password = ${q(reg.upstream_auth_password)}`)
      } else if (reg.upstream_auth_type === 'header') {
        if (reg.upstream_auth_header_name) lines.push(`name = ${q(reg.upstream_auth_header_name)}`)
        if (reg.upstream_auth_header_value) lines.push(`value = ${q(reg.upstream_auth_header_value)}`)
      }
    }
  }

  // [ip_blocking]
  if (ipBlocking.value.enabled) {
    lines.push('')
    lines.push('[ip_blocking]')
    lines.push(`enabled = true`)
    lines.push(`violation_threshold = ${ipBlocking.value.violation_threshold}`)
    lines.push(`violation_window_secs = ${ipBlocking.value.violation_window_secs}`)
    lines.push(`ban_duration_secs = ${ipBlocking.value.ban_duration_secs}`)
    lines.push(`trigger_on_status = ${listToToml(ipBlocking.value.trigger_on_status)}`)
  }

  // [otel]
  if (otel.value.enabled) {
    lines.push('')
    lines.push('[otel]')
    lines.push(`endpoint = ${q(otel.value.endpoint)}`)
    lines.push(`service_name = ${q(otel.value.service_name)}`)
  }

  return lines.join('\n')
})

// ── Actions ─────────────────────────────────────────────────────────────────

const copied = ref(false)
async function copyToml() {
  await navigator.clipboard.writeText(toml.value)
  copied.value = true
  setTimeout(() => { copied.value = false }, 1500)
}

function downloadToml() {
  const blob = new Blob([toml.value], { type: 'text/plain' })
  const a = document.createElement('a')
  a.href = URL.createObjectURL(blob)
  a.download = 'config.toml'
  a.click()
  URL.revokeObjectURL(a.href)
}

// Auth providers
function addAuthProvider() {
  authProviders.value.push({
    id: authSeq++,
    type: 'token',
    tokens: [],
    oidc_issuer: '',
    oidc_client_id: '',
    oidc_client_secret: '',
    oidc_redirect_uri: '',
    oidc_frontend_url: '',
    oidc_user_id_claim: 'sub',
    oidc_role_claim: 'role',
    k8s_api_server: '',
    k8s_audiences: 'batlehub',
  })
}
function removeAuthProvider(id: number) {
  authProviders.value = authProviders.value.filter(a => a.id !== id)
}
function addToken(auth: AuthProvider) {
  auth.tokens.push({ id: tokenSeq++, value: '', role: 'user', user_id: '' })
}
function removeToken(auth: AuthProvider, id: number) {
  auth.tokens = auth.tokens.filter(t => t.id !== id)
}

function addRegistry() {
  registries.value.push(defaultRegistry('npm'))
}
function removeRegistry(id: number) {
  registries.value = registries.value.filter(r => r.id !== id)
}
function onTypeChange(reg: Registry) {
  reg.upstreams = defaultUpstream[reg.type]
}

function addBackend() {
  storageBackends.value.push({
    id: backendSeq++,
    name: '',
    type: 'filesystem',
    path: './cache',
    bucket: '',
    region: 'us-east-1',
    endpoint_url: '',
    force_path_style: false,
    prefix: '',
  })
}
function removeBackend(id: number) {
  storageBackends.value = storageBackends.value.filter(b => b.id !== id)
}

const backendNames = computed(() =>
  storageBackends.value.map(b => b.name).filter(Boolean)
)

const isLocalOrHybrid = (reg: Registry) => reg.mode === 'local' || reg.mode === 'hybrid'

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
}`
}

const composerAuthSnippet = `{
  "http-basic": {
    "your-batlehub-host": {
      "username": "user",
      "password": "<your-token>"
    }
  }
}`
</script>

<template>
  <div class="cg-root">
    <!-- ── LEFT: form ──────────────────────────────────────────────────── -->
    <div class="cg-form">

      <!-- Server -->
      <section class="cg-section">
        <h3>Server</h3>
        <div class="cg-two-col">
          <label>Host<input v-model="server.host" placeholder="0.0.0.0" /></label>
          <label>Port<input v-model.number="server.port" type="number" min="1" max="65535" /></label>
        </div>
        <label>Static directory (optional)<input v-model="server.static_dir" placeholder="./ui/dist" /></label>
      </section>

      <!-- Database -->
      <section class="cg-section">
        <h3>Database</h3>
        <label>
          PostgreSQL URL
          <input v-model="database.url" placeholder="postgresql://batlehub:changeme@localhost:5432/batlehub" />
        </label>
        <label>
          Max connections
          <input v-model.number="database.max_connections" type="number" min="1" />
          <span class="cg-field-hint">Connection pool size (default: 10)</span>
        </label>
      </section>

      <!-- Metadata Cache -->
      <section class="cg-section">
        <h3>Metadata Cache</h3>
        <div class="cg-radio-row cg-mb">
          <label class="cg-radio"><input type="radio" v-model="metaCache.type" value="memory" /> Memory</label>
          <label class="cg-radio"><input type="radio" v-model="metaCache.type" value="postgres" /> PostgreSQL</label>
          <label class="cg-radio"><input type="radio" v-model="metaCache.type" value="redis" /> Redis</label>
        </div>
        <span class="cg-field-hint">
          <template v-if="metaCache.type === 'memory'">In-process cache — fast but lost on restart. Good for single-node dev deployments.</template>
          <template v-else-if="metaCache.type === 'postgres'">Persisted in the <code>metadata_cache</code> table — survives restarts, shared across replicas.</template>
          <template v-else>Persisted in Redis — survives restarts, shared across replicas.</template>
        </span>
        <template v-if="metaCache.type === 'redis'">
          <label style="margin-top:0.5rem">Redis URL<input v-model="metaCache.url" placeholder="redis://localhost:6379" /></label>
        </template>
      </section>

      <!-- Limits -->
      <section class="cg-section">
        <h3>Limits</h3>
        <label>
          Max artifact size (bytes)
          <input v-model="limits.max_artifact_size_bytes" placeholder="524288000  (500 MiB default)" />
          <span class="cg-field-hint">Applies to both proxy downloads and local publishes. Leave blank to use the 500 MiB default.</span>
        </label>
      </section>

      <!-- Storage -->
      <section class="cg-section">
        <h3>Storage</h3>
        <div class="cg-radio-row cg-mb">
          <label class="cg-radio"><input type="radio" v-model="storageMode" value="single" /> Single backend</label>
          <label class="cg-radio"><input type="radio" v-model="storageMode" value="multi" /> Multi-backend</label>
        </div>

        <!-- Single backend -->
        <template v-if="storageMode === 'single'">
          <div class="cg-radio-row cg-mb">
            <label class="cg-radio"><input type="radio" v-model="singleStorage.type" value="filesystem" /> Filesystem</label>
            <label class="cg-radio"><input type="radio" v-model="singleStorage.type" value="s3" /> S3 / RustFS</label>
          </div>
          <template v-if="singleStorage.type === 'filesystem'">
            <label>Cache path<input v-model="singleStorage.path" placeholder="./cache" /></label>
          </template>
          <template v-else>
            <div class="cg-two-col">
              <label>Bucket<input v-model="singleStorage.bucket" placeholder="my-artifacts" /></label>
              <label>Region<input v-model="singleStorage.region" placeholder="us-east-1" /></label>
            </div>
            <label>Endpoint URL (optional)<input v-model="singleStorage.endpoint_url" placeholder="http://minio:9000" /></label>
            <label>Key prefix (optional)<input v-model="singleStorage.prefix" placeholder="batlehub/" /><span class="cg-field-hint">Prepended to every object key — acts as a folder inside the bucket.</span></label>
            <label class="cg-check cg-mb">
              <input type="checkbox" v-model="singleStorage.force_path_style" />
              Force path-style URLs (required for MinIO, RustFS)
            </label>
          </template>
        </template>

        <!-- Multi-backend -->
        <template v-else>
          <label>Default backend name<input v-model="storageDefault" placeholder="primary" /></label>
          <div v-for="b in storageBackends" :key="b.id" class="cg-list-item">
            <div class="cg-two-col">
              <label>Backend name<input v-model="b.name" placeholder="primary" /></label>
              <label>
                Type
                <select v-model="b.type">
                  <option value="filesystem">Filesystem</option>
                  <option value="s3">S3 / RustFS</option>
                </select>
              </label>
            </div>
            <template v-if="b.type === 'filesystem'">
              <label>Cache path<input v-model="b.path" placeholder="./cache" /></label>
            </template>
            <template v-else>
              <div class="cg-two-col">
                <label>Bucket<input v-model="b.bucket" placeholder="my-artifacts" /></label>
                <label>Region<input v-model="b.region" placeholder="us-east-1" /></label>
              </div>
              <label>Endpoint URL (optional)<input v-model="b.endpoint_url" placeholder="http://minio:9000" /></label>
              <label>Key prefix (optional)<input v-model="b.prefix" placeholder="batlehub/" /><span class="cg-field-hint">Prepended to every object key.</span></label>
              <label class="cg-check cg-mb">
                <input type="checkbox" v-model="b.force_path_style" />
                Force path-style URLs (required for MinIO, RustFS)
              </label>
            </template>
            <button class="cg-btn-remove" @click="removeBackend(b.id)">Remove</button>
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
            </select>
          </label>

          <!-- Token auth -->
          <template v-if="auth.type === 'token'">
            <div v-for="tok in auth.tokens" :key="tok.id" class="cg-subitem">
              <label>Token value<input v-model="tok.value" placeholder="my-secret-token" /></label>
              <div class="cg-two-col">
                <label>
                  Role
                  <select v-model="tok.role">
                    <option value="admin">admin</option>
                    <option value="user">user</option>
                    <option value="anonymous">anonymous</option>
                  </select>
                </label>
                <label>User ID (optional)<input v-model="tok.user_id" placeholder="alice" /></label>
              </div>
              <button class="cg-btn-remove" @click="removeToken(auth, tok.id)">Remove token</button>
            </div>
            <button class="cg-btn-add" @click="addToken(auth)">+ Add token</button>
          </template>

          <!-- OIDC auth -->
          <template v-else-if="auth.type === 'oidc'">
            <label>Issuer URL<input v-model="auth.oidc_issuer" placeholder="https://accounts.example.com" /></label>
            <div class="cg-two-col">
              <label>Client ID<input v-model="auth.oidc_client_id" placeholder="batlehub" /></label>
              <label>Client secret<input v-model="auth.oidc_client_secret" type="password" placeholder="(optional for PKCE)" /></label>
            </div>
            <label>Redirect URI<input v-model="auth.oidc_redirect_uri" placeholder="https://batlehub.example.com/api/v1/auth/oidc/callback" /></label>
            <label>Frontend URL (dev only)<input v-model="auth.oidc_frontend_url" placeholder="http://localhost:5173" /><span class="cg-field-hint">Leave blank in production — the callback redirects to the same origin.</span></label>
            <div class="cg-two-col">
              <label>User ID claim<input v-model="auth.oidc_user_id_claim" placeholder="sub" /></label>
              <label>Role claim<input v-model="auth.oidc_role_claim" placeholder="role" /></label>
            </div>
          </template>

          <!-- Kubernetes auth -->
          <template v-else-if="auth.type === 'kubernetes'">
            <label>API server URL (optional)<input v-model="auth.k8s_api_server" placeholder="https://kubernetes.default.svc" /><span class="cg-field-hint">Leave blank to use the in-cluster environment variables.</span></label>
            <label>Audiences (comma-separated)<input v-model="auth.k8s_audiences" placeholder="batlehub" /></label>
          </template>

          <button class="cg-btn-remove" style="margin-top:0.5rem" @click="removeAuthProvider(auth.id)">Remove provider</button>
        </div>
        <button class="cg-btn-add" @click="addAuthProvider">+ Add auth provider</button>
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
                <option value="openvsx">OpenVSX</option>
                <option value="vscode-marketplace">VS Code Marketplace</option>
                <option value="goproxy">Go Modules</option>
                <option value="terraform">Terraform</option>
                <option value="github">GitHub</option>
              </select>
            </label>
          </div>
          <div class="cg-radio-row cg-mb">
            <label class="cg-radio"><input type="radio" v-model="reg.mode" value="proxy" /> proxy</label>
            <label class="cg-radio"><input type="radio" v-model="reg.mode" value="local" /> local</label>
            <label class="cg-radio"><input type="radio" v-model="reg.mode" value="hybrid" /> hybrid</label>
          </div>
          <label v-if="reg.mode !== 'local'">
            Upstreams (one per line)
            <textarea v-model="reg.upstreams" rows="2" />
          </label>

          <!-- Composer client config hint -->
          <div v-if="reg.type === 'composer'" class="cg-registry-hint">
            <p class="cg-hint-title">Composer client setup</p>
            <p class="cg-hint-text">Add a repository entry to your project's <code>composer.json</code>:</p>
            <pre class="cg-hint-code">{{ composerRepoSnippet(reg.name) }}</pre>
            <p class="cg-hint-text" style="margin-top:0.5rem">Store credentials in <code>auth.json</code> (never commit this file):</p>
            <pre class="cg-hint-code">{{ composerAuthSnippet }}</pre>
            <p class="cg-hint-text" style="margin-top:0.5rem">
              Publish via ZIP upload (must contain <code>composer.json</code> with <code>"name"</code> and <code>"version"</code>):
            </p>
            <pre class="cg-hint-code">curl -X POST \
  -H "Authorization: Bearer &lt;token&gt;" \
  -H "Content-Type: application/zip" \
  --data-binary @vendor-pkg-1.0.0.zip \
  "/proxy/{{ reg.name }}/api/upload"</pre>
          </div>

          <label v-if="storageMode === 'multi'">
            Storage backend (blank = use default)
            <select v-model="reg.storage_backend">
              <option value="">— default ({{ storageDefault }}) —</option>
              <option v-for="n in backendNames" :key="n" :value="n">{{ n }}</option>
            </select>
          </label>
          <p class="cg-perm-label">Permissions <span class="cg-perm-hint">(comma-separated; use <code>*</code> for all)</span></p>
          <div class="cg-three-col">
            <label>anonymous<input v-model="reg.rbac_anonymous" placeholder="" /></label>
            <label>user<input v-model="reg.rbac_user" placeholder="releases:read, source:read" /></label>
            <label>admin<input v-model="reg.rbac_admin" placeholder="*" /></label>
          </div>

          <!-- Advanced toggle -->
          <button class="cg-btn-advanced" @click="reg.showAdvanced = !reg.showAdvanced">
            {{ reg.showAdvanced ? '▲ Hide advanced' : '▼ Advanced options' }}
          </button>

          <div v-if="reg.showAdvanced" class="cg-advanced">

            <!-- Cache policy -->
            <p class="cg-subsection-label">Cache policy</p>
            <div class="cg-two-col">
              <label>Metadata TTL (s)<input v-model.number="reg.cache_metadata_ttl" type="number" min="0" /><span class="cg-field-hint">Default: 300 s</span></label>
              <label>Artifact TTL (s)<input v-model="reg.cache_artifact_ttl" placeholder="never" /><span class="cg-field-hint">Leave blank to keep forever</span></label>
            </div>
            <div class="cg-two-col">
              <label>Idle eviction (days)<input v-model="reg.cache_idle_days" placeholder="never" /></label>
              <label>Size cap (bytes)<input v-model="reg.cache_max_size_bytes" placeholder="no cap" /></label>
            </div>
            <label>Keep latest N versions<input v-model="reg.cache_keep_latest_n" placeholder="keep all" /></label>

            <!-- Rate limit -->
            <p class="cg-subsection-label">Rate limiting</p>
            <label class="cg-check cg-mb">
              <input type="checkbox" v-model="reg.rate_limit_enabled" /> Enable per-user rate limit
            </label>
            <template v-if="reg.rate_limit_enabled">
              <div class="cg-two-col">
                <label>Requests per window<input v-model.number="reg.rate_limit_rps" type="number" min="1" /></label>
                <label>Window (s)<input v-model.number="reg.rate_limit_window" type="number" min="1" /></label>
              </div>
              <label>
                Enforcement
                <select v-model="reg.rate_limit_enforcement">
                  <option value="block">block (429)</option>
                  <option value="warn">warn (header only)</option>
                </select>
              </label>
            </template>

            <!-- Quota (local/hybrid only) -->
            <template v-if="isLocalOrHybrid(reg)">
              <p class="cg-subsection-label">Publish quota</p>
              <label class="cg-check cg-mb">
                <input type="checkbox" v-model="reg.quota_enabled" /> Enable publish quota
              </label>
              <template v-if="reg.quota_enabled">
                <div class="cg-two-col">
                  <label>Max bytes per user<input v-model="reg.quota_max_bytes" placeholder="e.g. 1073741824" /></label>
                  <label>Max packages per user<input v-model="reg.quota_max_packages" placeholder="e.g. 100" /></label>
                </div>
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
                <input type="checkbox" v-model="reg.beta_channel_enabled" /> Gate pre-release versions to beta members
              </label>
            </template>

            <!-- Rules -->
            <p class="cg-subsection-label">Rules</p>
            <label class="cg-check">
              <input type="checkbox" v-model="reg.rule_age_gate_enabled" /> Release age gate
            </label>
            <template v-if="reg.rule_age_gate_enabled">
              <label>Min age (s)<input v-model.number="reg.rule_age_gate_min_age" type="number" min="0" /><span class="cg-field-hint">Reject downloads of packages younger than this many seconds.</span></label>
            </template>
            <label class="cg-check cg-mb">
              <input type="checkbox" v-model="reg.rule_deny_latest_enabled" /> Deny <code>@latest</code> / unpinned version requests
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
              <label>Token<input v-model="reg.upstream_auth_token" placeholder="ghp_..." /></label>
            </template>
            <template v-else-if="reg.upstream_auth_type === 'basic'">
              <div class="cg-two-col">
                <label>Username<input v-model="reg.upstream_auth_username" /></label>
                <label>Password<input v-model="reg.upstream_auth_password" type="password" /></label>
              </div>
            </template>
            <template v-else-if="reg.upstream_auth_type === 'header'">
              <div class="cg-two-col">
                <label>Header name<input v-model="reg.upstream_auth_header_name" placeholder="X-API-Key" /></label>
                <label>Header value<input v-model="reg.upstream_auth_header_value" /></label>
              </div>
            </template>

          </div>

          <button class="cg-btn-remove" @click="removeRegistry(reg.id)">Remove registry</button>
        </div>
        <button class="cg-btn-add" @click="addRegistry">+ Add registry</button>
      </section>

      <!-- IP Blocking -->
      <section class="cg-section">
        <h3>IP Blocking</h3>
        <label class="cg-check cg-mb">
          <input type="checkbox" v-model="ipBlocking.enabled" /> Enable fail2ban-style IP blocking
        </label>
        <template v-if="ipBlocking.enabled">
          <div class="cg-two-col">
            <label>Violation threshold<input v-model.number="ipBlocking.violation_threshold" type="number" min="1" /><span class="cg-field-hint">Violations before auto-block</span></label>
            <label>Window (s)<input v-model.number="ipBlocking.violation_window_secs" type="number" min="1" /></label>
          </div>
          <label>Ban duration (s)<input v-model.number="ipBlocking.ban_duration_secs" type="number" min="1" /></label>
          <label>Trigger on status codes<input v-model="ipBlocking.trigger_on_status" placeholder="429, 401" /><span class="cg-field-hint">Comma-separated HTTP status codes that count as violations.</span></label>
        </template>
      </section>

      <!-- OpenTelemetry -->
      <section class="cg-section">
        <h3>OpenTelemetry</h3>
        <label class="cg-check cg-mb">
          <input type="checkbox" v-model="otel.enabled" /> Enable tracing
        </label>
        <template v-if="otel.enabled">
          <label>OTLP gRPC endpoint<input v-model="otel.endpoint" placeholder="http://localhost:4317" /></label>
          <label>Service name<input v-model="otel.service_name" placeholder="batlehub" /></label>
        </template>
      </section>

    </div>

    <!-- ── RIGHT: live preview ─────────────────────────────────────────── -->
    <div class="cg-preview">
      <div class="cg-preview-header">
        <span class="cg-filename">config.toml</span>
        <div class="cg-actions">
          <button class="cg-btn-action" @click="copyToml">{{ copied ? 'Copied!' : 'Copy' }}</button>
          <button class="cg-btn-action" @click="downloadToml">Download</button>
        </div>
      </div>
      <pre class="cg-code"><code>{{ toml }}</code></pre>
    </div>
  </div>
</template>

<style scoped>
.cg-root {
  display: flex;
  gap: 2rem;
  align-items: flex-start;
  margin-top: 1.5rem;
  width: 100%;
}

/* ── Form column ────────────────────────────────────────────────────── */
.cg-form {
  flex: 1 1 0;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.cg-section {
  border: 1px solid var(--vp-c-divider);
  border-radius: 8px;
  padding: 1rem 1.2rem;
}

.cg-section h3 {
  margin: 0 0 0.75rem;
  font-size: 0.875rem;
  font-weight: 600;
  color: var(--vp-c-brand-1);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  border-bottom: 1px solid var(--vp-c-divider);
  padding-bottom: 0.4rem;
}

label {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  font-size: 0.84rem;
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
  border-radius: 5px;
  background: var(--vp-c-bg);
  color: var(--vp-c-text-1);
  font-size: 0.84rem;
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
  font-size: 0.84rem;
  color: var(--vp-c-text-1);
  cursor: pointer;
  margin-bottom: 0;
}

.cg-mb {
  margin-bottom: 0.6rem;
}

/* ── Grid layouts ────────────────────────────────────────────────── */
.cg-two-col {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0.75rem;
}

.cg-three-col {
  display: grid;
  grid-template-columns: 1fr 1fr 1fr;
  gap: 0.6rem;
}

/* ── List items ──────────────────────────────────────────────────── */
.cg-list-item {
  border: 1px solid var(--vp-c-divider);
  border-radius: 6px;
  padding: 0.75rem;
  margin-bottom: 0.75rem;
  background: var(--vp-c-bg-soft);
}

.cg-subitem {
  border: 1px solid var(--vp-c-divider);
  border-radius: 5px;
  padding: 0.6rem;
  margin-bottom: 0.5rem;
  background: var(--vp-c-bg);
}

/* ── Advanced panel ──────────────────────────────────────────────── */
.cg-advanced {
  border-top: 1px solid var(--vp-c-divider);
  margin-top: 0.75rem;
  padding-top: 0.75rem;
}

.cg-subsection-label {
  margin: 0.6rem 0 0.3rem;
  font-size: 0.8rem;
  font-weight: 600;
  color: var(--vp-c-text-2);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

/* ── Labels / hints ──────────────────────────────────────────────── */
.cg-perm-label {
  margin: 0.4rem 0 0.25rem;
  font-size: 0.84rem;
  color: var(--vp-c-text-2);
  font-weight: 500;
}

.cg-field-hint {
  font-size: 0.76rem;
  color: var(--vp-c-text-3);
  margin-top: 0.15rem;
}

.cg-field-hint code {
  font-size: 0.76rem;
}

.cg-perm-hint {
  font-size: 0.78rem;
  font-weight: 400;
  color: var(--vp-c-text-3);
}

/* ── Buttons ─────────────────────────────────────────────────────── */
.cg-btn-add {
  display: inline-block;
  padding: 0.3rem 0.8rem;
  font-size: 0.82rem;
  border: 1px dashed var(--vp-c-brand-2);
  border-radius: 5px;
  color: var(--vp-c-brand-1);
  background: transparent;
  cursor: pointer;
  transition: background 0.15s;
}
.cg-btn-add:hover {
  background: var(--vp-c-brand-soft);
}

.cg-btn-remove {
  font-size: 0.78rem;
  padding: 0.2rem 0.6rem;
  margin-top: 0.3rem;
  border: 1px solid var(--vp-c-danger-1, #e53e3e);
  border-radius: 4px;
  color: var(--vp-c-danger-1, #e53e3e);
  background: transparent;
  cursor: pointer;
  transition: background 0.15s;
}
.cg-btn-remove:hover {
  background: color-mix(in srgb, var(--vp-c-danger-1, #e53e3e) 10%, transparent);
}

.cg-btn-advanced {
  display: inline-block;
  margin: 0.5rem 0 0.25rem;
  padding: 0.2rem 0.6rem;
  font-size: 0.78rem;
  border: 1px solid var(--vp-c-divider);
  border-radius: 4px;
  background: transparent;
  color: var(--vp-c-text-2);
  cursor: pointer;
  transition: background 0.15s, border-color 0.15s;
}
.cg-btn-advanced:hover {
  border-color: var(--vp-c-brand-1);
  color: var(--vp-c-brand-1);
  background: var(--vp-c-brand-soft);
}

/* ── Preview column ──────────────────────────────────────────────── */
.cg-preview {
  flex: 0 0 560px;
  position: sticky;
  top: calc(var(--vp-nav-height) + 1rem);
  height: calc(100vh - var(--vp-nav-height) - 2rem);
  min-height: 480px;
  display: flex;
  flex-direction: column;
  border: 1px solid var(--vp-c-divider);
  border-radius: 8px;
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
  font-size: 0.82rem;
  font-family: var(--vp-font-family-mono);
  color: var(--vp-c-text-2);
}

.cg-actions {
  display: flex;
  gap: 0.5rem;
}

.cg-btn-action {
  font-size: 0.8rem;
  padding: 0.25rem 0.7rem;
  border: 1px solid var(--vp-c-divider);
  border-radius: 5px;
  background: var(--vp-c-bg);
  color: var(--vp-c-text-1);
  cursor: pointer;
  transition: border-color 0.15s, background 0.15s;
}
.cg-btn-action:hover {
  border-color: var(--vp-c-brand-1);
  background: var(--vp-c-brand-soft);
  color: var(--vp-c-brand-1);
}

.cg-code {
  margin: 0;
  padding: 0.9rem;
  font-size: 0.78rem;
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
  border-left: 3px solid var(--vp-c-brand-1);
  border-radius: 5px;
  padding: 0.65rem 0.8rem;
  margin: 0.5rem 0;
  background: var(--vp-c-brand-soft);
}

.cg-hint-title {
  font-size: 0.8rem;
  font-weight: 600;
  color: var(--vp-c-brand-1);
  margin: 0 0 0.35rem;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.cg-hint-text {
  font-size: 0.8rem;
  color: var(--vp-c-text-2);
  margin: 0 0 0.25rem;
}

.cg-hint-text code {
  font-size: 0.78rem;
}

.cg-hint-code {
  font-size: 0.76rem;
  font-family: var(--vp-font-family-mono);
  background: var(--vp-c-bg);
  border: 1px solid var(--vp-c-divider);
  border-radius: 4px;
  padding: 0.5rem 0.65rem;
  margin: 0.25rem 0 0;
  white-space: pre;
  overflow-x: auto;
  color: var(--vp-c-text-1);
  line-height: 1.5;
}

/* ── Responsive ──────────────────────────────────────────────────── */
@media (max-width: 1300px) {
  .cg-root {
    flex-direction: column;
  }
  .cg-preview {
    position: static;
    flex: none;
    width: 100%;
    height: 520px;
  }
  .cg-three-col {
    grid-template-columns: 1fr 1fr;
  }
}

@media (max-width: 600px) {
  .cg-two-col,
  .cg-three-col {
    grid-template-columns: 1fr;
  }
}
</style>
