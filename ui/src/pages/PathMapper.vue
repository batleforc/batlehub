<script setup lang="ts">
import { ref, computed, watch } from "vue";
import { API_BASE_URL } from "@/config";
import { listRegistries } from "@/client/sdk.gen";
import { useApi } from "@/composables/useApi";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";

// ── State ──────────────────────────────────────────────────────────────────────

const pastedUrl = ref("");
const registry = ref<"npm" | "cargo" | "github" | "composer">("github");

// Registry name overrides (default to type name for backward compat)
const githubRegistryName = ref("github");
const npmRegistryName = ref("npm");
const cargoRegistryName = ref("cargo");
const composerRegistryName = ref("composer");

const { data: registries } = useApi<Array<{ name: string; type: string }>>(
  () => listRegistries() as Promise<{ data?: unknown; error?: unknown }>,
  [],
);

watch(registries, (regs) => {
  if (!regs) return;
  const gh = regs.find((r) => r.type === "github");
  const np = regs.find((r) => r.type === "npm");
  const cg = regs.find((r) => r.type === "cargo");
  const cmp = regs.find((r) => r.type === "composer");
  if (gh) githubRegistryName.value = gh.name;
  if (np) npmRegistryName.value = np.name;
  if (cg) cargoRegistryName.value = cg.name;
  if (cmp) composerRegistryName.value = cmp.name;
});

const githubRegistries = computed(() => registries.value?.filter((r) => r.type === "github") ?? []);
const npmRegistries = computed(() => registries.value?.filter((r) => r.type === "npm") ?? []);
const cargoRegistries = computed(() => registries.value?.filter((r) => r.type === "cargo") ?? []);
const composerRegistries = computed(
  () => registries.value?.filter((r) => r.type === "composer") ?? [],
);

// npm fields
const npmPackage = ref("");
const npmVersion = ref("");

// cargo fields
const cargoName = ref("");
const cargoVersion = ref("");

// composer fields
const composerVendor = ref("");
const composerPackage = ref("");
const composerVersion = ref("");

// github fields
const ghOwner = ref("");
const ghRepo = ref("");
const ghRef = ref("");
const ghAssetId = ref("");
const ghFilename = ref("");
const ghFilePath = ref("");

// copy feedback
const copied = ref<string | null>(null);

// ── URL parser ─────────────────────────────────────────────────────────────────

/**
 * Whether `hostname` is `domain` itself or a proper subdomain of it.
 * Unlike `hostname.includes(domain)`, this rejects look-alike hosts such as
 * `evil-npmjs.org.attacker.com` or `notnpmjs.org`.
 */
function isHostOf(hostname: string, domain: string): boolean {
  return hostname === domain || hostname.endsWith(`.${domain}`);
}

function parseUrl(raw: string) {
  const str = raw.trim();
  if (!str) return;
  try {
    const u = new URL(str);
    const parts = u.pathname.split("/").filter(Boolean);

    if (isHostOf(u.hostname, "npmjs.org") || isHostOf(u.hostname, "npmjs.com")) {
      registry.value = "npm";
      if (parts[0]) npmPackage.value = decodeURIComponent(parts[0]);
      if (parts[1] && parts[1] !== "-") {
        npmVersion.value = parts[1];
      } else if (parts[1] === "-" && parts[2]) {
        const m = parts[2].match(/-(\d[\w.\-+]*)\.tgz$/);
        if (m) npmVersion.value = m[1];
      }
    } else if (isHostOf(u.hostname, "crates.io")) {
      registry.value = "cargo";
      const idx = parts.indexOf("crates");
      if (idx >= 0) {
        cargoName.value = parts[idx + 1] ?? "";
        const maybeVer = parts[idx + 2];
        cargoVersion.value = maybeVer && maybeVer !== "download" ? maybeVer : "";
      }
    } else if (u.hostname === "github.com") {
      registry.value = "github";
      ghOwner.value = parts[0] ?? "";
      ghRepo.value = parts[1] ?? "";
      if (parts[2] === "releases") {
        if (parts[3] === "tag" && parts[4]) ghRef.value = parts[4];
        if (parts[3] === "download" && parts[4]) {
          ghRef.value = parts[4];
          ghFilename.value = parts[5] ?? "";
        }
      } else if (parts[2] === "archive") {
        const last = parts[parts.length - 1];
        ghRef.value = last.replace(/\.(tar\.gz|zip)$/, "").replace(/^refs\/tags\//, "");
      } else if (parts[2] === "blob" && parts[3]) {
        // github.com/{owner}/{repo}/blob/{ref}/{path} — file browser URL
        ghRef.value = parts[3];
        ghFilePath.value = parts.slice(4).join("/");
      }
    } else if (u.hostname === "raw.githubusercontent.com") {
      registry.value = "github";
      ghOwner.value = parts[0] ?? "";
      ghRepo.value = parts[1] ?? "";
      ghRef.value = parts[2] ?? "";
      ghFilePath.value = parts.slice(3).join("/");
    } else if (u.hostname === "api.github.com") {
      registry.value = "github";
      if (parts[0] === "repos") {
        ghOwner.value = parts[1] ?? "";
        ghRepo.value = parts[2] ?? "";
        if (parts[3] === "releases" && parts[4] === "tags") ghRef.value = parts[5] ?? "";
        if (parts[3] === "releases" && parts[4] === "assets") ghAssetId.value = parts[5] ?? "";
      }
    } else if (u.hostname === "repo.packagist.org" || u.hostname === "packagist.org") {
      registry.value = "composer";
      if (parts[0] === "p2" && parts[1] && parts[2]) {
        // repo.packagist.org/p2/vendor/package.json
        composerVendor.value = parts[1];
        composerPackage.value = parts[2].replace(/\.json$/, "").replace(/~dev$/, "");
      } else if (parts[0] === "packages" && parts[1] && parts[2]) {
        // packagist.org/packages/vendor/package
        composerVendor.value = parts[1];
        composerPackage.value = parts[2];
      }
    }
  } catch {
    // not a valid URL — ignore silently
  }
}

watch(pastedUrl, parseUrl);

// ── Computed proxy paths ───────────────────────────────────────────────────────

interface ProxyPath {
  label: string;
  url: string;
  available: boolean;
}

const npmPaths = computed<ProxyPath[]>(() => {
  const reg = npmRegistryName.value.trim() || "npm";
  const pkg = npmPackage.value.trim();
  const ver = npmVersion.value.trim();
  if (!pkg) return [];
  return [
    { label: "Packument (all versions)", url: `/proxy/${reg}/${pkg}`, available: true },
    { label: "Version metadata", url: `/proxy/${reg}/${pkg}/${ver}`, available: !!ver },
    { label: "Tarball download", url: `/proxy/${reg}/${pkg}/${ver}/tarball`, available: !!ver },
  ];
});

const cargoPaths = computed<ProxyPath[]>(() => {
  const reg = cargoRegistryName.value.trim() || "cargo";
  const name = cargoName.value.trim();
  const ver = cargoVersion.value.trim();
  if (!name) return [];
  return [
    { label: "Crate metadata (all versions)", url: `/proxy/${reg}/${name}`, available: true },
    { label: "Version metadata", url: `/proxy/${reg}/${name}/${ver}`, available: !!ver },
    { label: ".crate download", url: `/proxy/${reg}/${name}/${ver}/download`, available: !!ver },
    { label: "Sparse index config", url: `/proxy/${reg}/registry/config.json`, available: true },
  ];
});

const githubPaths = computed<ProxyPath[]>(() => {
  const reg = githubRegistryName.value.trim() || "github";
  const owner = ghOwner.value.trim();
  const repo = ghRepo.value.trim();
  const ref = ghRef.value.trim();
  const asset = ghAssetId.value.trim();
  const filename = ghFilename.value.trim();
  const file = ghFilePath.value.trim();
  if (!owner || !repo) return [];
  const base = `${owner}/${repo}`;
  return [
    { label: "List releases", url: `/proxy/${reg}/${base}/releases`, available: true },
    {
      label: "Release by tag",
      url: `/proxy/${reg}/${base}/releases/tags/${ref}`,
      available: !!ref,
    },
    { label: "Source tarball", url: `/proxy/${reg}/${base}/tarball/${ref}`, available: !!ref },
    { label: "Zip archive", url: `/proxy/${reg}/${base}/zipball/${ref}`, available: !!ref },
    {
      label: "Asset by filename",
      url: `/proxy/${reg}/${base}/releases/download/${ref}/${filename}`,
      available: !!ref && !!filename,
    },
    {
      label: "Asset by ID",
      url: `/proxy/${reg}/${base}/releases/assets/${asset}`,
      available: !!asset,
    },
    {
      label: "Raw file",
      url: `/proxy/${reg}/${base}/raw/${ref}/${file}`,
      available: !!ref && !!file,
    },
  ];
});

const composerPaths = computed<ProxyPath[]>(() => {
  const reg = composerRegistryName.value.trim() || "composer";
  const vendor = composerVendor.value.trim();
  const pkg = composerPackage.value.trim();
  const ver = composerVersion.value.trim();
  const hasName = !!vendor && !!pkg;
  return [
    { label: "Root index", url: `/proxy/${reg}/packages.json`, available: true },
    {
      label: "Package metadata (p2)",
      url: `/proxy/${reg}/p2/${vendor}/${pkg}.json`,
      available: hasName,
    },
    {
      label: "Dev metadata (~dev)",
      url: `/proxy/${reg}/p2/${vendor}/${pkg}~dev.json`,
      available: hasName,
    },
    {
      label: "Dist download",
      url: `/proxy/${reg}/dist/${vendor}/${pkg}/${ver}`,
      available: hasName && !!ver,
    },
    { label: "Upload endpoint (POST)", url: `/proxy/${reg}/api/upload`, available: true },
    {
      label: "Yank version (DELETE)",
      url: `/proxy/${reg}/api/packages/${vendor}/${pkg}/versions/${ver}`,
      available: hasName && !!ver,
    },
  ];
});

const activePaths = computed(() =>
  registry.value === "npm"
    ? npmPaths.value
    : registry.value === "cargo"
      ? cargoPaths.value
      : registry.value === "composer"
        ? composerPaths.value
        : githubPaths.value,
);

// ── Copy helper ────────────────────────────────────────────────────────────────

async function copyUrl(path: string) {
  const full = `${API_BASE_URL}${path}`;
  await navigator.clipboard.writeText(full);
  copied.value = path;
  setTimeout(() => {
    copied.value = null;
  }, 1500);
}

function fullUrl(path: string) {
  return `${API_BASE_URL}${path}`;
}
</script>

<template>
  <div class="max-w-2xl space-y-6">
    <div>
      <h1 class="font-mono text-2xl font-bold cyber-text-glow">URL Mapper</h1>
      <p class="text-sm text-muted-foreground mt-1">
        Paste an upstream URL or fill in the fields to get the equivalent proxy path.
      </p>
    </div>

    <!-- Universal paste input -->
    <Card>
      <CardContent class="pt-5">
        <Label for="paste-url" class="text-xs uppercase tracking-wide text-muted-foreground">
          Paste an upstream URL to auto-fill
        </Label>
        <Input
          id="paste-url"
          v-model="pastedUrl"
          placeholder="https://registry.npmjs.org/lodash or https://github.com/owner/repo/…"
          class="mt-1.5 font-mono text-sm"
        />
      </CardContent>
    </Card>

    <!-- Registry tabs (plain buttons) -->
    <div class="flex gap-0.5 rounded-sm border border-border/50 bg-secondary p-0.5">
      <button
        v-for="tab in ['github', 'npm', 'cargo', 'composer'] as const"
        :key="tab"
        class="flex-1 rounded-sm py-1.5 font-mono text-sm font-medium transition-colors"
        :class="
          registry === tab
            ? 'bg-accent text-accent-foreground [box-shadow:var(--cyber-glow)] border border-primary/30'
            : 'text-muted-foreground hover:bg-accent/60 hover:text-accent-foreground'
        "
        @click="registry = tab"
      >
        {{
          tab === "github"
            ? "GitHub"
            : tab === "npm"
              ? "npm"
              : tab === "cargo"
                ? "Cargo"
                : "Composer"
        }}
      </button>
    </div>

    <!-- GitHub fields -->
    <div v-if="registry === 'github'" class="space-y-4">
      <div class="space-y-1">
        <Label for="gh-registry">Registry name</Label>
        <Input
          id="gh-registry"
          v-model="githubRegistryName"
          list="pm-github-list"
          placeholder="github"
          class="font-mono"
        />
        <datalist id="pm-github-list">
          <option v-for="r in githubRegistries" :key="r.name" :value="r.name" />
        </datalist>
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div class="space-y-1">
          <Label for="gh-owner">Owner</Label>
          <Input id="gh-owner" v-model="ghOwner" placeholder="batleforc" />
        </div>
        <div class="space-y-1">
          <Label for="gh-repo">Repository</Label>
          <Input id="gh-repo" v-model="ghRepo" placeholder="ProxyAuthK8S" />
        </div>
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div class="space-y-1">
          <Label for="gh-ref">Tag / branch / SHA</Label>
          <Input id="gh-ref" v-model="ghRef" placeholder="v0.1.9" />
        </div>
        <div class="space-y-1">
          <Label for="gh-asset"
            >Asset ID <span class="text-muted-foreground">(optional)</span></Label
          >
          <Input id="gh-asset" v-model="ghAssetId" placeholder="123456789" />
        </div>
      </div>
      <div class="space-y-1">
        <Label for="gh-filename"
          >Asset filename <span class="text-muted-foreground">(optional)</span></Label
        >
        <Input
          id="gh-filename"
          v-model="ghFilename"
          placeholder="tool-linux-amd64.tar.gz"
          class="font-mono"
        />
      </div>
      <div class="space-y-1">
        <Label for="gh-file"
          >Raw file path <span class="text-muted-foreground">(optional)</span></Label
        >
        <Input
          id="gh-file"
          v-model="ghFilePath"
          placeholder="README.md or path/to/file.yaml"
          class="font-mono"
        />
      </div>
    </div>

    <!-- npm fields -->
    <div v-else-if="registry === 'npm'" class="space-y-4">
      <div class="space-y-1">
        <Label for="npm-registry">Registry name</Label>
        <Input
          id="npm-registry"
          v-model="npmRegistryName"
          list="pm-npm-list"
          placeholder="npm"
          class="font-mono"
        />
        <datalist id="pm-npm-list">
          <option v-for="r in npmRegistries" :key="r.name" :value="r.name" />
        </datalist>
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div class="space-y-1">
          <Label for="npm-pkg">Package</Label>
          <Input id="npm-pkg" v-model="npmPackage" placeholder="lodash" class="font-mono" />
        </div>
        <div class="space-y-1">
          <Label for="npm-ver">Version <span class="text-muted-foreground">(optional)</span></Label>
          <Input id="npm-ver" v-model="npmVersion" placeholder="4.17.21" class="font-mono" />
        </div>
      </div>
    </div>

    <!-- Cargo fields -->
    <div v-else-if="registry === 'cargo'" class="space-y-4">
      <div class="space-y-1">
        <Label for="cargo-registry">Registry name</Label>
        <Input
          id="cargo-registry"
          v-model="cargoRegistryName"
          list="pm-cargo-list"
          placeholder="cargo"
          class="font-mono"
        />
        <datalist id="pm-cargo-list">
          <option v-for="r in cargoRegistries" :key="r.name" :value="r.name" />
        </datalist>
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div class="space-y-1">
          <Label for="cargo-name">Crate</Label>
          <Input id="cargo-name" v-model="cargoName" placeholder="serde" class="font-mono" />
        </div>
        <div class="space-y-1">
          <Label for="cargo-ver"
            >Version <span class="text-muted-foreground">(optional)</span></Label
          >
          <Input id="cargo-ver" v-model="cargoVersion" placeholder="1.0.197" class="font-mono" />
        </div>
      </div>
    </div>

    <!-- Composer fields -->
    <div v-else class="space-y-4">
      <div class="space-y-1">
        <Label for="composer-registry">Registry name</Label>
        <Input
          id="composer-registry"
          v-model="composerRegistryName"
          list="pm-composer-list"
          placeholder="composer"
          class="font-mono"
        />
        <datalist id="pm-composer-list">
          <option v-for="r in composerRegistries" :key="r.name" :value="r.name" />
        </datalist>
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div class="space-y-1">
          <Label for="composer-vendor">Vendor</Label>
          <Input
            id="composer-vendor"
            v-model="composerVendor"
            placeholder="symfony"
            class="font-mono"
          />
        </div>
        <div class="space-y-1">
          <Label for="composer-pkg">Package</Label>
          <Input
            id="composer-pkg"
            v-model="composerPackage"
            placeholder="console"
            class="font-mono"
          />
        </div>
      </div>
      <div class="space-y-1">
        <Label for="composer-ver"
          >Version
          <span class="text-muted-foreground">(optional — for dist download and yank)</span></Label
        >
        <Input id="composer-ver" v-model="composerVersion" placeholder="7.1.0" class="font-mono" />
      </div>
      <p class="text-xs text-muted-foreground">
        Package names follow the
        <code class="font-mono bg-muted px-1 rounded">vendor/package</code> convention. Paste a
        <code class="font-mono bg-muted px-1 rounded">packagist.org</code> or
        <code class="font-mono bg-muted px-1 rounded">repo.packagist.org</code> URL above to
        auto-fill.
      </p>
    </div>

    <!-- Results -->
    <div v-if="activePaths.length" class="space-y-2">
      <h2 class="text-sm font-medium text-muted-foreground uppercase tracking-wide">Proxy paths</h2>
      <div class="rounded-sm border divide-y">
        <div
          v-for="entry in activePaths"
          :key="entry.url"
          class="flex items-center gap-3 px-4 py-3"
          :class="entry.available ? '' : 'opacity-40'"
        >
          <span class="w-40 shrink-0 text-xs text-muted-foreground">{{ entry.label }}</span>
          <code class="flex-1 text-xs font-mono truncate" :title="fullUrl(entry.url)">
            {{ fullUrl(entry.url) }}
          </code>
          <Button
            v-if="entry.available"
            size="sm"
            variant="ghost"
            class="shrink-0 h-7 px-2 text-xs"
            @click="copyUrl(entry.url)"
          >
            {{ copied === entry.url ? "Copied!" : "Copy" }}
          </Button>
          <Badge v-else variant="outline" class="shrink-0 text-xs"> needs more fields </Badge>
        </div>
      </div>
    </div>

    <p v-else class="text-sm text-muted-foreground text-center py-4">
      Fill in the fields above to see the proxy paths.
    </p>
  </div>
</template>
