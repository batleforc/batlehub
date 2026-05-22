<script setup lang="ts">
import { ref, computed } from 'vue'

// ── Types ───────────────────────────────────────────────────────────────────

type RegistryMode = 'proxy' | 'local' | 'hybrid'
type RegistryType = 'npm' | 'cargo' | 'openvsx' | 'vscode-marketplace' | 'goproxy' | 'github'
type AuthRole = 'admin' | 'user' | 'anonymous'
type StorageBackendType = 'filesystem' | 's3'
type StorageMode = 'single' | 'multi'

interface StorageBackend {
  id: number
  name: string
  type: StorageBackendType
  // filesystem
  path: string
  // s3
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

interface Registry {
  id: number
  name: string
  type: RegistryType
  mode: RegistryMode
  upstreams: string
  storage_backend: string    // named backend or "" for default
  rbac_anonymous: string     // comma-separated permissions
  rbac_user: string
  rbac_admin: string
}

// ── State ───────────────────────────────────────────────────────────────────

const server = ref({ host: '0.0.0.0', port: 8080, static_dir: '' })
const database = ref({ url: '' })

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

// Auth tokens
let tokenSeq = 0
const tokens = ref<Token[]>([{ id: tokenSeq++, value: '', role: 'admin', user_id: 'admin' }])

// Registries
let registrySeq = 0
const registries = ref<Registry[]>([
  {
    id: registrySeq++,
    name: 'npm',
    type: 'npm',
    mode: 'proxy',
    upstreams: 'https://registry.npmjs.org',
    storage_backend: '',
    rbac_anonymous: 'releases:read, source:read',
    rbac_user: 'releases:read, source:read',
    rbac_admin: '*',
  },
])

// ── Helpers ─────────────────────────────────────────────────────────────────

function q(s: string) {
  return `"${s.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`
}

function permsToToml(csv: string): string {
  const perms = csv.split(',').map(p => p.trim()).filter(Boolean)
  if (!perms.length) return '[]'
  return `[${perms.map(q).join(', ')}]`
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

  // [[auth]] — token block
  const validTokens = tokens.value.filter(t => t.value.trim())
  if (validTokens.length) {
    lines.push('')
    lines.push('[[auth]]')
    lines.push(`type = "token"`)
    for (const tok of validTokens) {
      lines.push('')
      lines.push('[[auth.tokens]]')
      lines.push(`value = ${q(tok.value)}`)
      lines.push(`role = ${q(tok.role)}`)
      if (tok.user_id) lines.push(`user_id = ${q(tok.user_id)}`)
    }
  }

  // [storage] — single or multi-backend
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
    lines.push('')
    lines.push('[registries.rbac]')
    lines.push(`anonymous = ${permsToToml(reg.rbac_anonymous)}`)
    lines.push(`user = ${permsToToml(reg.rbac_user)}`)
    lines.push(`admin = ${permsToToml(reg.rbac_admin)}`)
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

function addToken() {
  tokens.value.push({ id: tokenSeq++, value: '', role: 'user', user_id: '' })
}
function removeToken(id: number) {
  tokens.value = tokens.value.filter(t => t.id !== id)
}

const defaultUpstream: Record<RegistryType, string> = {
  npm: 'https://registry.npmjs.org',
  cargo: 'https://index.crates.io',
  openvsx: 'https://open-vsx.org',
  'vscode-marketplace': 'https://marketplace.visualstudio.com',
  goproxy: 'https://proxy.golang.org',
  github: 'https://api.github.com',
}

function addRegistry() {
  registries.value.push({
    id: registrySeq++,
    name: '',
    type: 'npm',
    mode: 'proxy',
    upstreams: defaultUpstream['npm'],
    storage_backend: '',
    rbac_anonymous: 'releases:read, source:read',
    rbac_user: 'releases:read, source:read',
    rbac_admin: '*',
  })
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
            <label>Subfolder / key prefix (optional)<input v-model="singleStorage.prefix" placeholder="batlehub/" /><span class="cg-field-hint">Prepended to every object key — acts as a folder inside the bucket, e.g. <code>batlehub/</code></span></label>
            <label class="cg-check cg-mb">
              <input type="checkbox" v-model="singleStorage.force_path_style" />
              Force path-style URLs (required for MinIO, RustFS)
            </label>
          </template>
        </template>

        <!-- Multi-backend -->
        <template v-else>
          <label>Default backend name<input v-model="storageDefault" placeholder="primary" /></label>
          <div
            v-for="b in storageBackends"
            :key="b.id"
            class="cg-list-item"
          >
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
              <label>Subfolder / key prefix (optional)<input v-model="b.prefix" placeholder="batlehub/" /><span class="cg-field-hint">Prepended to every object key — acts as a folder inside the bucket, e.g. <code>batlehub/</code></span></label>
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

      <!-- Auth tokens -->
      <section class="cg-section">
        <h3>Auth tokens</h3>
        <div
          v-for="tok in tokens"
          :key="tok.id"
          class="cg-list-item"
        >
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
          <button class="cg-btn-remove" @click="removeToken(tok.id)">Remove</button>
        </div>
        <button class="cg-btn-add" @click="addToken">+ Add token</button>
      </section>

      <!-- Registries -->
      <section class="cg-section">
        <h3>Registries</h3>
        <div
          v-for="reg in registries"
          :key="reg.id"
          class="cg-list-item"
        >
          <div class="cg-two-col">
            <label>Name<input v-model="reg.name" placeholder="npm" /></label>
            <label>
              Type
              <select v-model="reg.type" @change="onTypeChange(reg)">
                <option value="npm">npm</option>
                <option value="cargo">Cargo</option>
                <option value="openvsx">OpenVSX</option>
                <option value="vscode-marketplace">VS Code Marketplace</option>
                <option value="goproxy">Go Modules</option>
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
          <!-- storage backend selector (multi-backend only) -->
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
          <button class="cg-btn-remove" @click="removeRegistry(reg.id)">Remove</button>
        </div>
        <button class="cg-btn-add" @click="addRegistry">+ Add registry</button>
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
  gap: 1.5rem;
  align-items: flex-start;
  margin-top: 1.5rem;
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

/* ── List items (tokens, registries, backends) ───────────────────── */
.cg-list-item {
  border: 1px solid var(--vp-c-divider);
  border-radius: 6px;
  padding: 0.75rem;
  margin-bottom: 0.75rem;
  background: var(--vp-c-bg-soft);
}

/* ── Permissions label ───────────────────────────────────────────── */
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

/* ── Preview column ──────────────────────────────────────────────── */
.cg-preview {
  flex: 0 0 400px;
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

/* ── Responsive ──────────────────────────────────────────────────── */
@media (max-width: 1100px) {
  .cg-root {
    flex-direction: column;
  }
  .cg-preview {
    position: static;
    flex: none;
    width: 100%;
    height: 420px;
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
