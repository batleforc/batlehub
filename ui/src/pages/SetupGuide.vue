<script setup lang="ts">
import { ref, computed, watch } from "vue";
import { RouterLink } from "vue-router";
import { API_BASE_URL } from "@/config";
import { listRegistries } from "@/client/sdk.gen";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import CodeBlock from "@/components/ui/code-block/CodeBlock.vue";
import {
  Card, CardHeader, CardTitle, CardDescription, CardContent,
} from "@/components/ui/card";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";

const base = computed(() => API_BASE_URL || window.location.origin);
const copied = ref<string | null>(null);

const { token, identity, isAuthenticated, expiresAt } = useAuth();

const netrcHost = computed(() => {
  try { return new URL(base.value).hostname; } catch { return base.value; }
});
const netrcLogin = computed(() => identity.value?.user_id ?? "token");
const netrcSnippet = computed(() =>
  `machine ${netrcHost.value}\nlogin ${netrcLogin.value}\npassword ${token.value}`
);
const isOidc = computed(() => expiresAt.value > 0);

const githubRegistryName    = ref("github");
const npmRegistryName       = ref("npm");
const cargoRegistryName     = ref("cargo");
const openvsxRegistryName   = ref("openvsx");
const goRegistryName        = ref("go");
const mavenRegistryName     = ref("maven");
const terraformRegistryName = ref("terraform");
const rubygemsRegistryName  = ref("rubygems");
const composerRegistryName  = ref("composer");

const { data: registries } = useApi<Array<{ name: string; type: string }>>(
  () => listRegistries() as Promise<{ data?: unknown; error?: unknown }>,
  [],
);

watch(registries, (regs) => {
  if (!regs) return;
  const gh  = regs.find(r => r.type === "github");
  const np  = regs.find(r => r.type === "npm");
  const cg  = regs.find(r => r.type === "cargo");
  const ovx = regs.find(r => r.type === "openvsx");
  const go  = regs.find(r => r.type === "goproxy");
  const mv  = regs.find(r => r.type === "maven");
  const tf  = regs.find(r => r.type === "terraform");
  const rg  = regs.find(r => r.type === "rubygems");
  const cmp = regs.find(r => r.type === "composer");
  if (gh)  githubRegistryName.value = gh.name;
  if (np)  npmRegistryName.value = np.name;
  if (cg)  cargoRegistryName.value = cg.name;
  if (ovx) openvsxRegistryName.value = ovx.name;
  if (go)  goRegistryName.value = go.name;
  if (mv)  mavenRegistryName.value = mv.name;
  if (tf)  terraformRegistryName.value = tf.name;
  if (rg)  rubygemsRegistryName.value = rg.name;
  if (cmp) composerRegistryName.value = cmp.name;
});

const githubRegistries    = computed(() => registries.value?.filter(r => r.type === "github")    ?? []);
const npmRegistries       = computed(() => registries.value?.filter(r => r.type === "npm")        ?? []);
const cargoRegistries     = computed(() => registries.value?.filter(r => r.type === "cargo")      ?? []);
const openvsxRegistries   = computed(() => registries.value?.filter(r => r.type === "openvsx")    ?? []);
const goRegistries        = computed(() => registries.value?.filter(r => r.type === "goproxy")    ?? []);
const mavenRegistries     = computed(() => registries.value?.filter(r => r.type === "maven")      ?? []);
const terraformRegistries = computed(() => registries.value?.filter(r => r.type === "terraform")  ?? []);
const rubygemsRegistries  = computed(() => registries.value?.filter(r => r.type === "rubygems")   ?? []);
const composerRegistries  = computed(() => registries.value?.filter(r => r.type === "composer")   ?? []);

async function copy(key: string, text: string) {
  await navigator.clipboard.writeText(text);
  copied.value = key;
  setTimeout(() => { copied.value = null; }, 1500);
}

// ── Config snippets ───────────────────────────────────────────────────────────

const miseSnippet = computed(() => {
  const b  = base.value;
  const gh = githubRegistryName.value || "github";
  const np = npmRegistryName.value || "npm";
  const cg = cargoRegistryName.value || "cargo";
  const lines: string[] = [];
  if (isAuthenticated.value) {
    lines.push(
      `# Authentication: mise reads ~/.netrc for HTTP Basic Auth`,
      `# machine ${netrcHost.value}`,
      `# login ${netrcLogin.value}`,
      `# password ${token.value}`,
      ``,
    );
  }
  lines.push(`[settings.url_replacements]`,
    ``,
    `# ── GitHub (registry: ${gh}) ─────────────────────────────────────────────────`,
    `# API (release listings, tag metadata, asset lists)`,
    `"regex:^https://api\\.github\\.com/repos/(.+)" = "${b}/proxy/${gh}/$1"`,
    ``,
    `# Release asset binaries (browser_download_url from API responses)`,
    `"regex:^https://github\\.com/([^/]+)/([^/]+)/releases/download/([^/]+)/(.+)" = "${b}/proxy/${gh}/$1/$2/releases/download/$3/$4"`,
    ``,
    `# Source tarballs`,
    `"regex:^https://github\\.com/([^/]+)/([^/]+)/archive/(?:refs/tags/)?(.+?)\\.tar\\.gz" = "${b}/proxy/${gh}/$1/$2/tarball/$3"`,
    `"regex:^https://codeload\\.github\\.com/([^/]+)/([^/]+)/tar\\.gz/(?:refs/tags/)?(.+)" = "${b}/proxy/${gh}/$1/$2/tarball/$3"`,
    ``,
    `# Zip archives`,
    `"regex:^https://github\\.com/([^/]+)/([^/]+)/archive/(?:refs/tags/)?(.+?)\\.zip" = "${b}/proxy/${gh}/$1/$2/zipball/$3"`,
    ``,
    `# Raw files (install scripts, manifests, …)`,
    `"regex:^https://raw\\.githubusercontent\\.com/([^/]+)/([^/]+)/([^/]+)/(.+)" = "${b}/proxy/${gh}/$1/$2/raw/$3/$4"`,
    ``,
    `# ── npm (registry: ${np}) ───────────────────────────────────────────────────`,
    `"regex:^https://registry\\.npmjs\\.org/(.+)" = "${b}/proxy/${np}/$1"`,
    ``,
    `# ── Cargo (registry: ${cg}) — downloads only, use .cargo/config.toml for full support`,
    `"regex:^https://static\\.crates\\.io/crates/([^/]+)/([^/]+)/.+\\.crate" = "${b}/proxy/${cg}/$1/$2/download"`,
  );
  return lines.join("\n");
});

const npmrcSnippet = computed(() => {
  const regUrl = `${base.value}/proxy/${npmRegistryName.value || "npm"}/`;
  const lines = [`registry=${regUrl}`];
  if (isAuthenticated.value) {
    try {
      const { host, pathname } = new URL(regUrl);
      lines.push(`//${host}${pathname}:_authToken=${token.value}`);
    } catch { /* skip */ }
  }
  return lines.join("\n");
});

const yarnSnippet = computed(() => {
  const lines = [`npmRegistryServer: "${base.value}/proxy/${npmRegistryName.value || "npm"}/"`];
  if (isAuthenticated.value) lines.push(`npmAuthToken: "${token.value}"`);
  return lines.join("\n");
});

const pnpmSnippet = computed(() => {
  const regUrl = `${base.value}/proxy/${npmRegistryName.value || "npm"}/`;
  const lines = [`registry=${regUrl}`];
  if (isAuthenticated.value) {
    try {
      const { host, pathname } = new URL(regUrl);
      lines.push(`//${host}${pathname}:_authToken=${token.value}`);
    } catch { /* skip */ }
  }
  return lines.join("\n");
});

const cargoSnippet = computed(() => {
  const b   = base.value;
  const reg = cargoRegistryName.value || "cargo";
  const lines = [
    `[source.crates-io]`,
    `replace-with = "batlehub"`,
    ``,
    `[source.batlehub]`,
    `registry = "sparse+${b}/proxy/${reg}/registry/"`,
  ];
  if (isAuthenticated.value) {
    lines.push(``, `[registries.batlehub]`, `token = "${token.value}"`);
  }
  return lines.join("\n");
});

const openvsxMiseSnippet = computed(() => {
  const b   = base.value;
  const reg = openvsxRegistryName.value || "openvsx";
  const lines: string[] = [];
  if (isAuthenticated.value) {
    lines.push(
      `# Authentication: mise reads ~/.netrc for HTTP Basic Auth`,
      `# machine ${netrcHost.value}`,
      `# login ${netrcLogin.value}`,
      `# password ${token.value}`,
      ``,
    );
  }
  lines.push(
    `[settings.url_replacements]`,
    ``,
    `# ── OpenVSX VSIX downloads ────────────────────────────────────────────────────`,
    `# Intercepts VSIX file downloads from open-vsx.org and routes them through the proxy.`,
    `# The extension ID is joined as publisher.name to match the proxy convention.`,
    `"regex:^https://open-vsx\\.org/api/([^/]+)/([^/]+)/([^/]+)/file/.+\\.vsix$" = "${b}/proxy/${reg}/$1.$2/$3/vsix"`,
  );
  return lines.join("\n");
});

const openvsxDirectSnippet = computed(() => {
  const b   = base.value;
  const reg = openvsxRegistryName.value || "openvsx";
  return `${b}/proxy/${reg}/{publisher}.{extension}/{version}/vsix`;
});

const goSnippet = computed(() => {
  const b   = base.value;
  const reg = goRegistryName.value || "go";
  let proxyUrl = `${b}/proxy/${reg}`;
  if (isAuthenticated.value) {
    try {
      const u = new URL(`${b}/proxy/${reg}`);
      u.username = netrcLogin.value;
      u.password = token.value ?? "";
      proxyUrl = u.toString();
    } catch { /* keep original */ }
  }
  return [
    `# Shell / CI environment — set before running go commands`,
    `export GONOSUMCHECK="*"`,
    `export GONOSUMDB="*"`,
    `export GOPROXY="${proxyUrl},direct"`,
  ].join("\n");
});

const openvsxVscodiumSnippet = computed(() => {
  const b   = base.value;
  const reg = openvsxRegistryName.value || "openvsx";
  return [
    `// ~/.config/VSCodium/User/product.json  (or merge into existing product.json)`,
    `{`,
    `  "extensionGallery": {`,
    `    "serviceUrl": "${b}/proxy/${reg}/vscode/gallery",`,
    `    "itemUrl": "${b}/proxy/${reg}/vscode/item",`,
    `    "resourceUrlTemplate": "${b}/proxy/${reg}/vscode/unpkg/{publisher}/{name}/{version}/{path}"`,
    `  }`,
    `}`,
  ].join("\n");
});

const mavenSettingsSnippet = computed(() => {
  const b   = base.value;
  const reg = mavenRegistryName.value || "maven";
  const lines = [`<!-- ~/.m2/settings.xml -->`];
  if (isAuthenticated.value) {
    lines.push(
      `<settings>`,
      `  <servers>`,
      `    <server>`,
      `      <id>batlehub-${reg}</id>`,
      `      <username>${netrcLogin.value}</username>`,
      `      <password>${token.value}</password>`,
      `    </server>`,
      `  </servers>`,
      `  <mirrors>`,
      `    <mirror>`,
      `      <id>batlehub-${reg}</id>`,
      `      <name>BatleHub Maven Proxy</name>`,
      `      <url>${b}/proxy/${reg}/maven2/</url>`,
      `      <mirrorOf>*</mirrorOf>`,
      `    </mirror>`,
      `  </mirrors>`,
      `</settings>`,
    );
  } else {
    lines.push(
      `<settings>`,
      `  <mirrors>`,
      `    <mirror>`,
      `      <id>batlehub-${reg}</id>`,
      `      <name>BatleHub Maven Proxy</name>`,
      `      <url>${b}/proxy/${reg}/maven2/</url>`,
      `      <mirrorOf>*</mirrorOf>`,
      `    </mirror>`,
      `  </mirrors>`,
      `</settings>`,
    );
  }
  return lines.join("\n");
});

const mavenPublishSnippet = computed(() => {
  const b   = base.value;
  const reg = mavenRegistryName.value || "maven";
  const lines = [
    `<!-- pom.xml — add <distributionManagement> inside <project> -->`,
    `<distributionManagement>`,
    `  <repository>`,
    `    <id>batlehub-${reg}</id>`,
    `    <url>${b}/proxy/${reg}/maven2/</url>`,
    `  </repository>`,
    `</distributionManagement>`,
    ``,
    `<!-- Then publish with: -->`,
    `<!-- mvn deploy -->`,
  ];
  return lines.join("\n");
});

const terraformrcSnippet = computed(() => {
  const b   = base.value;
  const reg = terraformRegistryName.value || "terraform";
  let hostPart = b;
  try { hostPart = new URL(b).hostname; } catch { /* keep b */ }
  const lines = [
    `# ~/.terraformrc`,
    `provider_installation {`,
    `  network_mirror {`,
    `    url = "${b}/proxy/${reg}/"`,
    `  }`,
    `}`,
  ];
  if (isAuthenticated.value) {
    lines.push(
      ``,
      `credentials "${hostPart}" {`,
      `  token = "${token.value}"`,
      `}`,
    );
  }
  return lines.join("\n");
});

const gemsrcSnippet = computed(() => {
  const b   = base.value;
  const reg = rubygemsRegistryName.value || "rubygems";
  let proxyUrl = `${b}/proxy/${reg}/`;
  if (isAuthenticated.value) {
    try {
      const u = new URL(`${b}/proxy/${reg}/`);
      u.username = netrcLogin.value;
      u.password = token.value ?? "";
      proxyUrl = u.toString();
    } catch { /* keep original */ }
  }
  return [
    `# Bundler — mirror rubygems.org through the proxy`,
    `# Run once, or commit to .bundle/config`,
    `bundle config set mirror.https://rubygems.org/ ${proxyUrl}`,
    ``,
    `# gem CLI — replace the default source`,
    `# gem sources --remove https://rubygems.org/`,
    `# gem sources --add ${proxyUrl}`,
  ].join("\n");
});

const gemPublishSnippet = computed(() => {
  const b   = base.value;
  const reg = rubygemsRegistryName.value || "rubygems";
  const lines = [
    `# Publish a gem (local / hybrid mode only)`,
    `gem push name-version.gem --host ${b}/proxy/${reg}`,
  ];
  if (isAuthenticated.value) {
    lines.push(``, `# Credentials: set GEM_HOST_API_KEY or pass --key`);
    lines.push(`export GEM_HOST_API_KEY="${token.value}"`);
  }
  return lines.join("\n");
});

const terraformModuleSnippet = computed(() => {
  const b   = base.value;
  const reg = terraformRegistryName.value || "terraform";
  return [
    `# Upload a module (tar.gz archive)`,
    `curl -X POST \\`,
    `  -H "Authorization: Bearer ${isAuthenticated.value ? token.value : "<your-token>"}" \\`,
    `  -H "Content-Type: application/gzip" \\`,
    `  --data-binary @module.tar.gz \\`,
    `  "${b}/proxy/${reg}/v1/modules/namespace/name/provider/1.0.0"`,
    ``,
    `# Download artifact URL returned as X-Terraform-Get header:`,
    `# ${b}/proxy/${reg}/v1/modules/namespace/name/provider/1.0.0/artifact`,
  ].join("\n");
});

// ── Composer snippets ─────────────────────────────────────────────────────────

const composerJsonSnippet = computed(() => {
  const b   = base.value;
  const reg = composerRegistryName.value || "composer";
  const lines = [
    `// composer.json — add inside the root object`,
    `"repositories": [`,
    `  {`,
    `    "type": "composer",`,
    `    "url": "${b}/proxy/${reg}/",`,
  ];
  if (isAuthenticated.value) {
    lines.push(
      `    "options": {`,
      `      "http": {`,
      `        "header": ["Authorization: Bearer ${token.value}"]`,
      `      }`,
      `    }`,
    );
  }
  lines.push(`  }`, `]`);
  return lines.join("\n");
});

const composerAuthSnippet = computed(() => {
  let hostPart = base.value;
  try { hostPart = new URL(base.value).hostname; } catch { /* keep */ }
  return [
    `// auth.json — project root or ~/.config/composer/auth.json`,
    `// Never commit this file!`,
    `{`,
    `  "http-basic": {`,
    `    "${hostPart}": {`,
    `      "username": "${isAuthenticated.value ? (netrcLogin.value ?? "user") : "user"}",`,
    `      "password": "${isAuthenticated.value ? token.value : "<your-token>"}"`,
    `    }`,
    `  }`,
    `}`,
  ].join("\n");
});

const composerPublishSnippet = computed(() => {
  const b   = base.value;
  const reg = composerRegistryName.value || "composer";
  const tok = isAuthenticated.value ? token.value : "<your-token>";
  return [
    `# Publish a package (Local / Hybrid mode only)`,
    `# ZIP must contain composer.json with "name" (vendor/pkg) and "version"`,
    `zip -r vendor-pkg-1.0.0.zip vendor-pkg-1.0.0/`,
    ``,
    `curl -X POST \\`,
    `  -H "Authorization: Bearer ${tok}" \\`,
    `  -H "Content-Type: application/zip" \\`,
    `  --data-binary @vendor-pkg-1.0.0.zip \\`,
    `  "${b}/proxy/${reg}/api/upload"`,
    ``,
    `# Yank a version`,
    `curl -X DELETE \\`,
    `  -H "Authorization: Bearer ${tok}" \\`,
    `  "${b}/proxy/${reg}/api/packages/vendor/pkg/versions/1.0.0"`,
  ].join("\n");
});
</script>

<template>
  <div class="max-w-7xl space-y-8">
    <div>
      <h1 class="text-2xl font-semibold">Setup Guide</h1>
      <p class="text-sm text-muted-foreground mt-1">
        Configure your tools to route package downloads through this proxy.
        Snippets are pre-filled with this server's address.
      </p>
    </div>

    <!-- ── Registry names ── -->
    <Card>
      <CardHeader>
        <CardTitle>Registry names</CardTitle>
        <CardDescription class="mt-1">
          Enter the registry names from your <code class="text-xs font-mono bg-muted px-1 rounded">config.toml</code>.
          All snippets below update automatically.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div class="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-9">
          <div class="space-y-1">
            <Label for="sg-github">GitHub registry</Label>
            <Input id="sg-github" v-model="githubRegistryName" list="sg-github-list" placeholder="github" class="font-mono text-sm" />
            <datalist id="sg-github-list">
              <option v-for="r in githubRegistries" :key="r.name" :value="r.name" />
            </datalist>
          </div>
          <div class="space-y-1">
            <Label for="sg-npm">npm registry</Label>
            <Input id="sg-npm" v-model="npmRegistryName" list="sg-npm-list" placeholder="npm" class="font-mono text-sm" />
            <datalist id="sg-npm-list">
              <option v-for="r in npmRegistries" :key="r.name" :value="r.name" />
            </datalist>
          </div>
          <div class="space-y-1">
            <Label for="sg-cargo">Cargo registry</Label>
            <Input id="sg-cargo" v-model="cargoRegistryName" list="sg-cargo-list" placeholder="cargo" class="font-mono text-sm" />
            <datalist id="sg-cargo-list">
              <option v-for="r in cargoRegistries" :key="r.name" :value="r.name" />
            </datalist>
          </div>
          <div class="space-y-1">
            <Label for="sg-openvsx">OpenVSX registry</Label>
            <Input id="sg-openvsx" v-model="openvsxRegistryName" list="sg-openvsx-list" placeholder="openvsx" class="font-mono text-sm" />
            <datalist id="sg-openvsx-list">
              <option v-for="r in openvsxRegistries" :key="r.name" :value="r.name" />
            </datalist>
          </div>
          <div class="space-y-1">
            <Label for="sg-go">Go registry</Label>
            <Input id="sg-go" v-model="goRegistryName" list="sg-go-list" placeholder="go" class="font-mono text-sm" />
            <datalist id="sg-go-list">
              <option v-for="r in goRegistries" :key="r.name" :value="r.name" />
            </datalist>
          </div>
          <div class="space-y-1">
            <Label for="sg-maven">Maven registry</Label>
            <Input id="sg-maven" v-model="mavenRegistryName" list="sg-maven-list" placeholder="maven" class="font-mono text-sm" />
            <datalist id="sg-maven-list">
              <option v-for="r in mavenRegistries" :key="r.name" :value="r.name" />
            </datalist>
          </div>
          <div class="space-y-1">
            <Label for="sg-terraform">Terraform registry</Label>
            <Input id="sg-terraform" v-model="terraformRegistryName" list="sg-terraform-list" placeholder="terraform" class="font-mono text-sm" />
            <datalist id="sg-terraform-list">
              <option v-for="r in terraformRegistries" :key="r.name" :value="r.name" />
            </datalist>
          </div>
          <div class="space-y-1">
            <Label for="sg-rubygems">RubyGems registry</Label>
            <Input id="sg-rubygems" v-model="rubygemsRegistryName" list="sg-rubygems-list" placeholder="rubygems" class="font-mono text-sm" />
            <datalist id="sg-rubygems-list">
              <option v-for="r in rubygemsRegistries" :key="r.name" :value="r.name" />
            </datalist>
          </div>
          <div class="space-y-1">
            <Label for="sg-composer">Composer registry</Label>
            <Input id="sg-composer" v-model="composerRegistryName" list="sg-composer-list" placeholder="composer" class="font-mono text-sm" />
            <datalist id="sg-composer-list">
              <option v-for="r in composerRegistries" :key="r.name" :value="r.name" />
            </datalist>
          </div>
        </div>
      </CardContent>
    </Card>

    <!-- ── Tool config tabs ── -->
    <Tabs default-value="mise">
      <TabsList :class="isAuthenticated ? 'grid grid-cols-10' : 'grid grid-cols-9'">
        <TabsTrigger value="mise">mise</TabsTrigger>
        <TabsTrigger value="npm">npm</TabsTrigger>
        <TabsTrigger value="cargo">Cargo</TabsTrigger>
        <TabsTrigger value="openvsx">OpenVSX</TabsTrigger>
        <TabsTrigger value="go">Go</TabsTrigger>
        <TabsTrigger value="maven">Maven</TabsTrigger>
        <TabsTrigger value="terraform">Terraform</TabsTrigger>
        <TabsTrigger value="rubygems">RubyGems</TabsTrigger>
        <TabsTrigger value="composer">Composer</TabsTrigger>
        <TabsTrigger v-if="isAuthenticated" value="netrc">.netrc</TabsTrigger>
      </TabsList>

      <!-- mise -->
      <TabsContent value="mise">
        <Card>
          <CardHeader>
            <div class="flex items-center justify-between">
              <CardDescription>
                URL replacements intercept all HTTP requests made by mise
                (aqua, ubi, and other backends). Add to your global
                <code class="text-xs font-mono bg-muted px-1 rounded">~/.config/mise/config.toml</code>
                or a project-local <code class="text-xs font-mono bg-muted px-1 rounded">mise.toml</code>.
              </CardDescription>
              <Badge variant="outline" class="shrink-0 font-mono text-xs ml-4">mise.toml</Badge>
            </div>
          </CardHeader>
          <CardContent>
            <CodeBlock :code="miseSnippet" lang="toml">
              <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('mise', miseSnippet)">
                {{ copied === 'mise' ? 'Copied!' : 'Copy' }}
              </Button>
            </CodeBlock>
          </CardContent>
        </Card>
      </TabsContent>

      <!-- npm / yarn / pnpm -->
      <TabsContent value="npm">
        <Card>
          <CardHeader>
            <div class="flex items-center justify-between">
              <CardDescription>
                Sets the registry for all packages. Place in your project root or
                <code class="text-xs font-mono bg-muted px-1 rounded">~/.npmrc</code>
                for global use.
              </CardDescription>
              <Badge variant="outline" class="shrink-0 font-mono text-xs ml-4">.npmrc</Badge>
            </div>
          </CardHeader>
          <CardContent class="space-y-4">
            <div>
              <p class="text-xs text-muted-foreground mb-1.5">npm / npm workspaces</p>
              <CodeBlock :code="npmrcSnippet" lang="ini">
                <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('npmrc', npmrcSnippet)">
                  {{ copied === 'npmrc' ? 'Copied!' : 'Copy' }}
                </Button>
              </CodeBlock>
            </div>
            <div>
              <p class="text-xs text-muted-foreground mb-1.5">Yarn Berry — <code class="font-mono">.yarnrc.yml</code></p>
              <CodeBlock :code="yarnSnippet" lang="yaml">
                <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('yarn', yarnSnippet)">
                  {{ copied === 'yarn' ? 'Copied!' : 'Copy' }}
                </Button>
              </CodeBlock>
            </div>
            <div>
              <p class="text-xs text-muted-foreground mb-1.5">pnpm — <code class="font-mono">.npmrc</code></p>
              <CodeBlock :code="pnpmSnippet" lang="ini">
                <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('pnpm', pnpmSnippet)">
                  {{ copied === 'pnpm' ? 'Copied!' : 'Copy' }}
                </Button>
              </CodeBlock>
            </div>
            <p class="text-xs text-muted-foreground">
              To route only a specific scope through the proxy, use
              <code class="font-mono bg-muted px-1 rounded">@myorg:registry={{ base }}/proxy/npm/</code> instead.
            </p>
          </CardContent>
        </Card>
      </TabsContent>

      <!-- Cargo -->
      <TabsContent value="cargo">
        <Card>
          <CardHeader>
            <div class="flex items-center justify-between">
              <CardDescription>
                Replaces crates.io as the default source. Cargo fetches the sparse
                index and <code class="text-xs font-mono bg-muted px-1 rounded">.crate</code>
                files through the proxy. Add to your project's
                <code class="text-xs font-mono bg-muted px-1 rounded">.cargo/config.toml</code>
                or the global
                <code class="text-xs font-mono bg-muted px-1 rounded">~/.cargo/config.toml</code>.
              </CardDescription>
              <Badge variant="outline" class="shrink-0 font-mono text-xs ml-4">.cargo/config.toml</Badge>
            </div>
          </CardHeader>
          <CardContent class="space-y-3">
            <CodeBlock :code="cargoSnippet" lang="toml">
              <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('cargo', cargoSnippet)">
                {{ copied === 'cargo' ? 'Copied!' : 'Copy' }}
              </Button>
            </CodeBlock>
            <p class="text-xs text-muted-foreground">
              The proxy implements the
              <a
                href="https://doc.rust-lang.org/cargo/reference/registry-protocols.html#sparse-protocol"
                target="_blank"
                rel="noopener"
                class="underline underline-offset-2 hover:text-foreground transition-colors"
              >sparse registry protocol</a>.
              Checksums from the index match the cached
              <code class="font-mono bg-muted px-1 rounded">.crate</code> files,
              so <code class="font-mono bg-muted px-1 rounded">cargo verify-project</code> continues to work.
            </p>
          </CardContent>
        </Card>
      </TabsContent>

      <!-- OpenVSX -->
      <TabsContent value="openvsx">
        <Card>
          <CardHeader>
            <div class="flex items-center justify-between">
              <CardDescription>
                Proxy VS Code extension downloads from
                <a href="https://open-vsx.org" target="_blank" rel="noopener" class="underline underline-offset-2 hover:text-foreground transition-colors">open-vsx.org</a>.
                Extension IDs follow the <code class="text-xs font-mono bg-muted px-1 rounded">publisher.name</code> convention.
              </CardDescription>
              <Badge variant="outline" class="shrink-0 font-mono text-xs ml-4">OpenVSX</Badge>
            </div>
          </CardHeader>
          <CardContent class="space-y-4">
            <div>
              <p class="text-xs text-muted-foreground mb-1.5">Direct VSIX download URL</p>
              <CodeBlock :code="openvsxDirectSnippet" lang="text">
                <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('openvsx-direct', openvsxDirectSnippet)">
                  {{ copied === 'openvsx-direct' ? 'Copied!' : 'Copy' }}
                </Button>
              </CodeBlock>
              <p class="text-xs text-muted-foreground mt-1.5">
                Example: download and install via CLI —
                <code class="font-mono bg-muted px-1 rounded">curl -L {proxy}/ms-python.python/2024.0.0/vsix -o ext.vsix &amp;&amp; code --install-extension ext.vsix</code>
              </p>
            </div>
            <div>
              <p class="text-xs text-muted-foreground mb-1.5">mise — URL replacement to intercept VSIX downloads</p>
              <CodeBlock :code="openvsxMiseSnippet" lang="toml">
                <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('openvsx-mise', openvsxMiseSnippet)">
                  {{ copied === 'openvsx-mise' ? 'Copied!' : 'Copy' }}
                </Button>
              </CodeBlock>
            </div>
            <div>
              <p class="text-xs text-muted-foreground mb-1.5">VSCodium / Code - OSS — extension gallery (<code class="font-mono">product.json</code>)</p>
              <CodeBlock :code="openvsxVscodiumSnippet" lang="jsonc">
                <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('openvsx-vscodium', openvsxVscodiumSnippet)">
                  {{ copied === 'openvsx-vscodium' ? 'Copied!' : 'Copy' }}
                </Button>
              </CodeBlock>
              <p class="text-xs text-muted-foreground mt-1.5">
                Requires the proxy to implement the VS Code gallery protocol
                (<code class="font-mono bg-muted px-1 rounded">/vscode/gallery</code> endpoints). Only VSIX proxying is supported today.
              </p>
              <p v-if="isAuthenticated" class="text-xs text-muted-foreground mt-1.5">
                VSCodium does not support HTTP Basic Auth in <code class="font-mono bg-muted px-1 rounded">product.json</code>.
                Add your credentials to <code class="font-mono bg-muted px-1 rounded">~/.netrc</code> — see the <strong>.netrc</strong> tab.
              </p>
            </div>
          </CardContent>
        </Card>
      </TabsContent>

      <!-- Go -->
      <TabsContent value="go">
        <Card>
          <CardHeader>
            <div class="flex items-center justify-between">
              <CardDescription>
                Set <code class="text-xs font-mono bg-muted px-1 rounded">GOPROXY</code> to route
                Go module downloads through this proxy. Modules are cached after the first download.
                Append <code class="text-xs font-mono bg-muted px-1 rounded">,direct</code> so the
                go tool falls back to the original source when the proxy returns 404.
              </CardDescription>
              <Badge variant="outline" class="shrink-0 font-mono text-xs ml-4">Go</Badge>
            </div>
          </CardHeader>
          <CardContent class="space-y-4">
            <div>
              <p class="text-xs text-muted-foreground mb-1.5">Environment variables</p>
              <CodeBlock :code="goSnippet" lang="bash">
                <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('go', goSnippet)">
                  {{ copied === 'go' ? 'Copied!' : 'Copy' }}
                </Button>
              </CodeBlock>
            </div>
            <p class="text-xs text-muted-foreground">
              The proxy implements the
              <a
                href="https://go.dev/ref/mod#goproxy-protocol"
                target="_blank"
                rel="noopener"
                class="underline underline-offset-2 hover:text-foreground transition-colors"
              >GOPROXY protocol</a>.
              Module zip archives are cached permanently after first download.
              <code class="font-mono bg-muted px-1 rounded">@latest</code> and
              <code class="font-mono bg-muted px-1 rounded">@v/list</code> responses are also cached —
              clear the proxy storage if you need to pick up newly published versions immediately.
            </p>
          </CardContent>
        </Card>
      </TabsContent>

      <!-- Maven -->
      <TabsContent value="maven">
        <Card>
          <CardHeader>
            <div class="flex items-center justify-between">
              <CardDescription>
                Route Maven/Gradle dependency downloads through this proxy, or publish private
                artifacts (<code class="text-xs font-mono bg-muted px-1 rounded">mvn deploy</code>)
                when the registry is configured in <code class="text-xs font-mono bg-muted px-1 rounded">Local</code>
                or <code class="text-xs font-mono bg-muted px-1 rounded">Hybrid</code> mode.
              </CardDescription>
              <Badge variant="outline" class="shrink-0 font-mono text-xs ml-4">Maven</Badge>
            </div>
          </CardHeader>
          <CardContent class="space-y-4">
            <div>
              <p class="text-xs text-muted-foreground mb-1.5">
                <code class="font-mono">~/.m2/settings.xml</code> — proxy all Maven dependencies
              </p>
              <CodeBlock :code="mavenSettingsSnippet" lang="xml">
                <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('maven-settings', mavenSettingsSnippet)">
                  {{ copied === 'maven-settings' ? 'Copied!' : 'Copy' }}
                </Button>
              </CodeBlock>
            </div>
            <div>
              <p class="text-xs text-muted-foreground mb-1.5">
                <code class="font-mono">pom.xml</code> — publish private artifacts (Local / Hybrid mode)
              </p>
              <CodeBlock :code="mavenPublishSnippet" lang="xml">
                <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('maven-publish', mavenPublishSnippet)">
                  {{ copied === 'maven-publish' ? 'Copied!' : 'Copy' }}
                </Button>
              </CodeBlock>
              <p class="text-xs text-muted-foreground mt-1.5">
                The registry must be configured with
                <code class="font-mono bg-muted px-1 rounded">mode = "local"</code> or
                <code class="font-mono bg-muted px-1 rounded">mode = "hybrid"</code> in
                <code class="font-mono bg-muted px-1 rounded">config.toml</code> to accept publishes.
                The <code class="font-mono bg-muted px-1 rounded">server</code> id in
                <code class="font-mono bg-muted px-1 rounded">settings.xml</code> must match the
                <code class="font-mono bg-muted px-1 rounded">repository id</code> in
                <code class="font-mono bg-muted px-1 rounded">distributionManagement</code>.
              </p>
            </div>
          </CardContent>
        </Card>
      </TabsContent>

      <!-- Terraform -->
      <TabsContent value="terraform">
        <Card>
          <CardHeader>
            <div class="flex items-center justify-between">
              <CardDescription>
                Proxy Terraform provider downloads via network mirror, or publish private modules
                and providers when the registry is configured in
                <code class="text-xs font-mono bg-muted px-1 rounded">Local</code>
                or <code class="text-xs font-mono bg-muted px-1 rounded">Hybrid</code> mode.
              </CardDescription>
              <Badge variant="outline" class="shrink-0 font-mono text-xs ml-4">Terraform</Badge>
            </div>
          </CardHeader>
          <CardContent class="space-y-4">
            <div>
              <p class="text-xs text-muted-foreground mb-1.5">
                <code class="font-mono">~/.terraformrc</code> — provider network mirror
              </p>
              <CodeBlock :code="terraformrcSnippet" lang="hcl">
                <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('terraformrc', terraformrcSnippet)">
                  {{ copied === 'terraformrc' ? 'Copied!' : 'Copy' }}
                </Button>
              </CodeBlock>
              <p class="text-xs text-muted-foreground mt-1.5">
                The <code class="font-mono bg-muted px-1 rounded">network_mirror</code> block
                redirects all provider downloads through this proxy.
                Providers are cached after first download in Proxy/Hybrid mode,
                or served entirely locally in Local mode.
              </p>
            </div>
            <div>
              <p class="text-xs text-muted-foreground mb-1.5">Upload a private module (Local / Hybrid mode)</p>
              <CodeBlock :code="terraformModuleSnippet" lang="bash">
                <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('terraform-module', terraformModuleSnippet)">
                  {{ copied === 'terraform-module' ? 'Copied!' : 'Copy' }}
                </Button>
              </CodeBlock>
              <p class="text-xs text-muted-foreground mt-1.5">
                The response includes an
                <code class="font-mono bg-muted px-1 rounded">X-Terraform-Get</code>
                header pointing to the artifact download URL.
                Modules can also be yanked via the admin API.
              </p>
            </div>
          </CardContent>
        </Card>
      </TabsContent>

      <!-- RubyGems -->
      <TabsContent value="rubygems">
        <Card>
          <CardHeader>
            <div class="flex items-center justify-between">
              <CardDescription>
                Mirror rubygems.org through this proxy for Bundler and the gem CLI.
                Gems are cached after the first download. Publish private gems with
                <code class="text-xs font-mono bg-muted px-1 rounded">gem push</code>
                when the registry is configured in
                <code class="text-xs font-mono bg-muted px-1 rounded">Local</code>
                or <code class="text-xs font-mono bg-muted px-1 rounded">Hybrid</code> mode.
              </CardDescription>
              <Badge variant="outline" class="shrink-0 font-mono text-xs ml-4">RubyGems</Badge>
            </div>
          </CardHeader>
          <CardContent class="space-y-4">
            <div>
              <p class="text-xs text-muted-foreground mb-1.5">Bundler mirror / gem CLI source</p>
              <CodeBlock :code="gemsrcSnippet" lang="bash">
                <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('gemsrc', gemsrcSnippet)">
                  {{ copied === 'gemsrc' ? 'Copied!' : 'Copy' }}
                </Button>
              </CodeBlock>
              <p class="text-xs text-muted-foreground mt-1.5">
                The <code class="font-mono bg-muted px-1 rounded">bundle config</code> mirror setting
                intercepts all rubygems.org requests transparently — no changes to your
                <code class="font-mono bg-muted px-1 rounded">Gemfile</code> needed.
              </p>
            </div>
            <div>
              <p class="text-xs text-muted-foreground mb-1.5">Publish a private gem (Local / Hybrid mode)</p>
              <CodeBlock :code="gemPublishSnippet" lang="bash">
                <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('gem-publish', gemPublishSnippet)">
                  {{ copied === 'gem-publish' ? 'Copied!' : 'Copy' }}
                </Button>
              </CodeBlock>
              <p class="text-xs text-muted-foreground mt-1.5">
                The registry must be configured with
                <code class="font-mono bg-muted px-1 rounded">mode = "local"</code> or
                <code class="font-mono bg-muted px-1 rounded">mode = "hybrid"</code> in
                <code class="font-mono bg-muted px-1 rounded">config.toml</code> to accept publishes.
              </p>
            </div>
          </CardContent>
        </Card>
      </TabsContent>

      <!-- Composer -->
      <TabsContent value="composer">
        <Card>
          <CardHeader>
            <div class="flex items-center justify-between">
              <CardDescription>
                Proxy PHP Composer package downloads from
                <a href="https://packagist.org" target="_blank" rel="noopener" class="underline underline-offset-2 hover:text-foreground transition-colors">Packagist</a>
                or publish private packages via ZIP upload when the registry is configured in
                <code class="text-xs font-mono bg-muted px-1 rounded">Local</code>
                or <code class="text-xs font-mono bg-muted px-1 rounded">Hybrid</code> mode.
                Authentication uses <code class="text-xs font-mono bg-muted px-1 rounded">auth.json</code>
                (HTTP Basic) rather than a token header — this is a Composer convention.
              </CardDescription>
              <Badge variant="outline" class="shrink-0 font-mono text-xs ml-4">Composer</Badge>
            </div>
          </CardHeader>
          <CardContent class="space-y-4">
            <div>
              <p class="text-xs text-muted-foreground mb-1.5">
                <code class="font-mono">composer.json</code> — add the proxy as a repository
              </p>
              <CodeBlock :code="composerJsonSnippet" lang="jsonc">
                <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('composer-json', composerJsonSnippet)">
                  {{ copied === 'composer-json' ? 'Copied!' : 'Copy' }}
                </Button>
              </CodeBlock>
            </div>
            <div>
              <p class="text-xs text-muted-foreground mb-1.5">
                <code class="font-mono">auth.json</code> — credentials (never commit this file)
              </p>
              <CodeBlock :code="composerAuthSnippet" lang="jsonc">
                <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('composer-auth', composerAuthSnippet)">
                  {{ copied === 'composer-auth' ? 'Copied!' : 'Copy' }}
                </Button>
              </CodeBlock>
              <p class="text-xs text-muted-foreground mt-1.5">
                Place <code class="font-mono bg-muted px-1 rounded">auth.json</code> in your project root or
                <code class="font-mono bg-muted px-1 rounded">~/.config/composer/auth.json</code> for global use.
                When present, Composer sends HTTP Basic credentials automatically — no
                <code class="font-mono bg-muted px-1 rounded">options.http.header</code> needed in
                <code class="font-mono bg-muted px-1 rounded">composer.json</code>.
              </p>
            </div>
            <div>
              <p class="text-xs text-muted-foreground mb-1.5">Publish a private package (Local / Hybrid mode)</p>
              <CodeBlock :code="composerPublishSnippet" lang="bash">
                <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('composer-publish', composerPublishSnippet)">
                  {{ copied === 'composer-publish' ? 'Copied!' : 'Copy' }}
                </Button>
              </CodeBlock>
              <p class="text-xs text-muted-foreground mt-1.5">
                The ZIP must contain a valid <code class="font-mono bg-muted px-1 rounded">composer.json</code>
                at its root or inside a single top-level directory (GitHub archive layout).
                The <code class="font-mono bg-muted px-1 rounded">name</code> field must use the
                <code class="font-mono bg-muted px-1 rounded">vendor/package</code> format and the
                <code class="font-mono bg-muted px-1 rounded">version</code> field determines the published version.
              </p>
            </div>
          </CardContent>
        </Card>
      </TabsContent>

      <!-- .netrc (authenticated only) -->
      <TabsContent v-if="isAuthenticated" value="netrc">
        <Card>
          <CardHeader>
            <div class="flex items-center justify-between">
              <CardDescription>
                Credentials for tools that use HTTP Basic Auth (curl, wget, …).
                Place in <code class="text-xs font-mono bg-muted px-1 rounded">~/.netrc</code>
                and restrict permissions with
                <code class="text-xs font-mono bg-muted px-1 rounded">chmod 600 ~/.netrc</code>.
              </CardDescription>
              <Badge variant="outline" class="shrink-0 font-mono text-xs ml-4">~/.netrc</Badge>
            </div>
          </CardHeader>
          <CardContent class="space-y-3">
            <CodeBlock :code="netrcSnippet" lang="text">
              <Button size="sm" variant="ghost" class="absolute top-2 right-2 h-7 px-2 text-xs" @click="copy('netrc', netrcSnippet)">
                {{ copied === 'netrc' ? 'Copied!' : 'Copy' }}
              </Button>
            </CodeBlock>
            <p v-if="isOidc" class="text-xs text-muted-foreground">
              Your current token is a short-lived OIDC session token.
              For long-lived automation, create a
              <RouterLink to="/tokens" class="underline underline-offset-2 hover:text-foreground transition-colors">personal API token</RouterLink>
              and use that as the password.
            </p>
          </CardContent>
        </Card>
      </TabsContent>
    </Tabs>
  </div>
</template>
