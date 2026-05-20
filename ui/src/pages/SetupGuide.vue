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

const githubRegistryName  = ref("github");
const npmRegistryName     = ref("npm");
const cargoRegistryName   = ref("cargo");
const openvsxRegistryName = ref("openvsx");
const goRegistryName      = ref("go");

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
  if (gh)  githubRegistryName.value = gh.name;
  if (np)  npmRegistryName.value = np.name;
  if (cg)  cargoRegistryName.value = cg.name;
  if (ovx) openvsxRegistryName.value = ovx.name;
  if (go)  goRegistryName.value = go.name;
});

const githubRegistries  = computed(() => registries.value?.filter(r => r.type === "github")   ?? []);
const npmRegistries     = computed(() => registries.value?.filter(r => r.type === "npm")       ?? []);
const cargoRegistries   = computed(() => registries.value?.filter(r => r.type === "cargo")     ?? []);
const openvsxRegistries = computed(() => registries.value?.filter(r => r.type === "openvsx")   ?? []);
const goRegistries      = computed(() => registries.value?.filter(r => r.type === "goproxy")   ?? []);

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
  return [
    `[settings.url_replacements]`,
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
  ].join("\n");
});

const npmrcSnippet = computed(() => `registry=${base.value}/proxy/${npmRegistryName.value || "npm"}/`);

const yarnSnippet = computed(() => `npmRegistryServer: "${base.value}/proxy/${npmRegistryName.value || "npm"}/"`);

const pnpmSnippet = computed(() => `registry=${base.value}/proxy/${npmRegistryName.value || "npm"}/`);

const cargoSnippet = computed(() => {
  const b   = base.value;
  const reg = cargoRegistryName.value || "cargo";
  return [
    `[source.crates-io]`,
    `replace-with = "batlehub"`,
    ``,
    `[source.batlehub]`,
    `registry = "sparse+${b}/proxy/${reg}/registry/"`,
  ].join("\n");
});

const openvsxMiseSnippet = computed(() => {
  const b   = base.value;
  const reg = openvsxRegistryName.value || "openvsx";
  return [
    `[settings.url_replacements]`,
    ``,
    `# ── OpenVSX VSIX downloads ────────────────────────────────────────────────────`,
    `# Intercepts VSIX file downloads from open-vsx.org and routes them through the proxy.`,
    `# The extension ID is joined as publisher.name to match the proxy convention.`,
    `"regex:^https://open-vsx\\.org/api/([^/]+)/([^/]+)/([^/]+)/file/.+\\.vsix$" = "${b}/proxy/${reg}/$1.$2/$3/vsix"`,
  ].join("\n");
});

const openvsxDirectSnippet = computed(() => {
  const b   = base.value;
  const reg = openvsxRegistryName.value || "openvsx";
  return `${b}/proxy/${reg}/{publisher}.{extension}/{version}/vsix`;
});

const goSnippet = computed(() => {
  const b   = base.value;
  const reg = goRegistryName.value || "go";
  return [
    `# Shell / CI environment — set before running go commands`,
    `export GONOSUMCHECK="*"`,
    `export GONOSUMDB="*"`,
    `export GOPROXY="${b}/proxy/${reg},direct"`,
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
</script>

<template>
  <div class="max-w-3xl space-y-8">
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
        <div class="grid grid-cols-2 gap-3 sm:grid-cols-5">
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
        </div>
      </CardContent>
    </Card>

    <!-- ── Tool config tabs ── -->
    <Tabs default-value="mise">
      <TabsList :class="isAuthenticated ? 'grid grid-cols-6' : 'grid grid-cols-5'">
        <TabsTrigger value="mise">mise</TabsTrigger>
        <TabsTrigger value="npm">npm</TabsTrigger>
        <TabsTrigger value="cargo">Cargo</TabsTrigger>
        <TabsTrigger value="openvsx">OpenVSX</TabsTrigger>
        <TabsTrigger value="go">Go</TabsTrigger>
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
