<script setup lang="ts">
import { ref, computed, watch } from "vue";
import { API_BASE_URL } from "@/config";
import { listRegistries } from "@/client/sdk.gen";
import { useApi } from "@/composables/useApi";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Card, CardHeader, CardTitle, CardDescription, CardContent,
} from "@/components/ui/card";

const base = computed(() => API_BASE_URL || window.location.origin);
const copied = ref<string | null>(null);

const githubRegistryName = ref("github");
const npmRegistryName    = ref("npm");
const cargoRegistryName  = ref("cargo");

const { data: registries } = useApi<Array<{ name: string; type: string }>>(
  () => listRegistries() as Promise<{ data?: unknown; error?: unknown }>,
  [],
);

watch(registries, (regs) => {
  if (!regs) return;
  const gh = regs.find(r => r.type === "github");
  const np = regs.find(r => r.type === "npm");
  const cg = regs.find(r => r.type === "cargo");
  if (gh) githubRegistryName.value = gh.name;
  if (np) npmRegistryName.value = np.name;
  if (cg) cargoRegistryName.value = cg.name;
});

const githubRegistries = computed(() => registries.value?.filter(r => r.type === "github") ?? []);
const npmRegistries    = computed(() => registries.value?.filter(r => r.type === "npm") ?? []);
const cargoRegistries  = computed(() => registries.value?.filter(r => r.type === "cargo") ?? []);

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
    `replace-with = "proxy-cache"`,
    ``,
    `[source.proxy-cache]`,
    `registry = "sparse+${b}/proxy/${reg}/registry/"`,
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
        <div class="grid grid-cols-3 gap-3">
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
        </div>
      </CardContent>
    </Card>

    <!-- ── mise ── -->
    <Card>
      <CardHeader>
        <div class="flex items-center justify-between">
          <div>
            <CardTitle>mise</CardTitle>
            <CardDescription class="mt-1">
              URL replacements intercept all HTTP requests made by mise
              (aqua, ubi, and other backends). Add to your global
              <code class="text-xs font-mono bg-muted px-1 rounded">~/.config/mise/config.toml</code>
              or a project-local <code class="text-xs font-mono bg-muted px-1 rounded">mise.toml</code>.
            </CardDescription>
          </div>
          <Badge variant="outline" class="shrink-0 font-mono text-xs">mise.toml</Badge>
        </div>
      </CardHeader>
      <CardContent class="space-y-2">
        <div class="relative">
          <pre class="bg-muted rounded-md p-4 text-xs font-mono overflow-x-auto leading-relaxed">{{ miseSnippet }}</pre>
          <Button
            size="sm"
            variant="ghost"
            class="absolute top-2 right-2 h-7 px-2 text-xs"
            @click="copy('mise', miseSnippet)"
          >
            {{ copied === 'mise' ? 'Copied!' : 'Copy' }}
          </Button>
        </div>
      </CardContent>
    </Card>

    <!-- ── npm ── -->
    <Card>
      <CardHeader>
        <div class="flex items-center justify-between">
          <div>
            <CardTitle>npm</CardTitle>
            <CardDescription class="mt-1">
              Sets the registry for all packages. Place in your project root or
              <code class="text-xs font-mono bg-muted px-1 rounded">~/.npmrc</code>
              for global use.
            </CardDescription>
          </div>
          <Badge variant="outline" class="shrink-0 font-mono text-xs">.npmrc</Badge>
        </div>
      </CardHeader>
      <CardContent class="space-y-4">
        <div>
          <p class="text-xs text-muted-foreground mb-1.5">npm / npm workspaces</p>
          <div class="relative">
            <pre class="bg-muted rounded-md p-4 text-xs font-mono overflow-x-auto">{{ npmrcSnippet }}</pre>
            <Button
              size="sm"
              variant="ghost"
              class="absolute top-2 right-2 h-7 px-2 text-xs"
              @click="copy('npmrc', npmrcSnippet)"
            >
              {{ copied === 'npmrc' ? 'Copied!' : 'Copy' }}
            </Button>
          </div>
        </div>
        <div>
          <p class="text-xs text-muted-foreground mb-1.5">Yarn Berry — <code class="font-mono">.yarnrc.yml</code></p>
          <div class="relative">
            <pre class="bg-muted rounded-md p-4 text-xs font-mono overflow-x-auto">{{ yarnSnippet }}</pre>
            <Button
              size="sm"
              variant="ghost"
              class="absolute top-2 right-2 h-7 px-2 text-xs"
              @click="copy('yarn', yarnSnippet)"
            >
              {{ copied === 'yarn' ? 'Copied!' : 'Copy' }}
            </Button>
          </div>
        </div>
        <div>
          <p class="text-xs text-muted-foreground mb-1.5">pnpm — <code class="font-mono">.npmrc</code></p>
          <div class="relative">
            <pre class="bg-muted rounded-md p-4 text-xs font-mono overflow-x-auto">{{ pnpmSnippet }}</pre>
            <Button
              size="sm"
              variant="ghost"
              class="absolute top-2 right-2 h-7 px-2 text-xs"
              @click="copy('pnpm', pnpmSnippet)"
            >
              {{ copied === 'pnpm' ? 'Copied!' : 'Copy' }}
            </Button>
          </div>
        </div>
        <p class="text-xs text-muted-foreground">
          To route only a specific scope through the proxy, use
          <code class="font-mono bg-muted px-1 rounded">@myorg:registry={{ base }}/proxy/npm/</code> instead.
        </p>
      </CardContent>
    </Card>

    <!-- ── Cargo ── -->
    <Card>
      <CardHeader>
        <div class="flex items-center justify-between">
          <div>
            <CardTitle>Cargo</CardTitle>
            <CardDescription class="mt-1">
              Replaces crates.io as the default source. Cargo fetches the sparse
              index and <code class="text-xs font-mono bg-muted px-1 rounded">.crate</code>
              files through the proxy. Add to your project's
              <code class="text-xs font-mono bg-muted px-1 rounded">.cargo/config.toml</code>
              or the global
              <code class="text-xs font-mono bg-muted px-1 rounded">~/.cargo/config.toml</code>.
            </CardDescription>
          </div>
          <Badge variant="outline" class="shrink-0 font-mono text-xs">.cargo/config.toml</Badge>
        </div>
      </CardHeader>
      <CardContent class="space-y-3">
        <div class="relative">
          <pre class="bg-muted rounded-md p-4 text-xs font-mono overflow-x-auto leading-relaxed">{{ cargoSnippet }}</pre>
          <Button
            size="sm"
            variant="ghost"
            class="absolute top-2 right-2 h-7 px-2 text-xs"
            @click="copy('cargo', cargoSnippet)"
          >
            {{ copied === 'cargo' ? 'Copied!' : 'Copy' }}
          </Button>
        </div>
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
  </div>
</template>
