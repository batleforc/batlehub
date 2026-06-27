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
const registry = ref<
  | "npm"
  | "cargo"
  | "github"
  | "composer"
  | "nuget"
  | "pypi"
  | "goproxy"
  | "maven"
  | "terraform"
  | "rubygems"
  | "conda"
  | "openvsx"
  | "forgejo"
  | "gitlab"
  | "deb"
  | "rpm"
  | "pacman"
  | "jetbrains"
>("github");

// Registry name overrides (default to type name for backward compat)
const githubRegistryName = ref("github");
const npmRegistryName = ref("npm");
const cargoRegistryName = ref("cargo");
const composerRegistryName = ref("composer");
const nugetRegistryName = ref("nuget");
const pypiRegistryName = ref("pypi");
const goproxyRegistryName = ref("goproxy");
const mavenRegistryName = ref("maven");
const terraformRegistryName = ref("terraform");
const rubygemsRegistryName = ref("rubygems");
const condaRegistryName = ref("conda");
const openvsxRegistryName = ref("openvsx");
const forgejoRegistryName = ref("forgejo");
const gitlabRegistryName = ref("gitlab");
const debRegistryName = ref("deb");
const rpmRegistryName = ref("rpm");
const pacmanRegistryName = ref("pacman");
const jetbrainsRegistryName = ref("jetbrains");

const { data: registries } = useApi<Array<{ name: string; type: string }>>(
  () => listRegistries() as Promise<{ data?: unknown; error?: unknown }>,
  [],
);

watch(registries, (regs) => {
  if (!regs) return;
  const pick = (type: string) => regs.find((r) => r.type === type);
  const gh = pick("github");
  const np = pick("npm");
  const cg = pick("cargo");
  const cmp = pick("composer");
  const nug = pick("nuget");
  const pyp = pick("pypi");
  const go = pick("goproxy");
  const mvn = pick("maven");
  const tf = pick("terraform");
  const rg = pick("rubygems");
  const cnd = pick("conda");
  const vsx = pick("openvsx");
  const fj = pick("forgejo");
  const gl = pick("gitlab");
  const db = pick("deb");
  const rp = pick("rpm");
  const pc = pick("pacman");
  const jb = pick("jetbrains");
  if (gh) githubRegistryName.value = gh.name;
  if (np) npmRegistryName.value = np.name;
  if (cg) cargoRegistryName.value = cg.name;
  if (cmp) composerRegistryName.value = cmp.name;
  if (nug) nugetRegistryName.value = nug.name;
  if (pyp) pypiRegistryName.value = pyp.name;
  if (go) goproxyRegistryName.value = go.name;
  if (mvn) mavenRegistryName.value = mvn.name;
  if (tf) terraformRegistryName.value = tf.name;
  if (rg) rubygemsRegistryName.value = rg.name;
  if (cnd) condaRegistryName.value = cnd.name;
  if (vsx) openvsxRegistryName.value = vsx.name;
  if (fj) forgejoRegistryName.value = fj.name;
  if (gl) gitlabRegistryName.value = gl.name;
  if (db) debRegistryName.value = db.name;
  if (rp) rpmRegistryName.value = rp.name;
  if (pc) pacmanRegistryName.value = pc.name;
  if (jb) jetbrainsRegistryName.value = jb.name;
});

const githubRegistries = computed(() => registries.value?.filter((r) => r.type === "github") ?? []);
const npmRegistries = computed(() => registries.value?.filter((r) => r.type === "npm") ?? []);
const cargoRegistries = computed(() => registries.value?.filter((r) => r.type === "cargo") ?? []);
const composerRegistries = computed(() => registries.value?.filter((r) => r.type === "composer") ?? []);
const nugetRegistries = computed(() => registries.value?.filter((r) => r.type === "nuget") ?? []);
const pypiRegistries = computed(() => registries.value?.filter((r) => r.type === "pypi") ?? []);
const goproxyRegistries = computed(() => registries.value?.filter((r) => r.type === "goproxy") ?? []);
const mavenRegistries = computed(() => registries.value?.filter((r) => r.type === "maven") ?? []);
const terraformRegistries = computed(() => registries.value?.filter((r) => r.type === "terraform") ?? []);
const rubygemsRegistries = computed(() => registries.value?.filter((r) => r.type === "rubygems") ?? []);
const condaRegistries = computed(() => registries.value?.filter((r) => r.type === "conda") ?? []);
const openvsxRegistries = computed(() => registries.value?.filter((r) => r.type === "openvsx") ?? []);
const forgejoRegistries = computed(() => registries.value?.filter((r) => r.type === "forgejo") ?? []);
const gitlabRegistries = computed(() => registries.value?.filter((r) => r.type === "gitlab") ?? []);
const debRegistries = computed(() => registries.value?.filter((r) => r.type === "deb") ?? []);
const rpmRegistries = computed(() => registries.value?.filter((r) => r.type === "rpm") ?? []);
const pacmanRegistries = computed(() => registries.value?.filter((r) => r.type === "pacman") ?? []);
const jetbrainsRegistries = computed(() => registries.value?.filter((r) => r.type === "jetbrains") ?? []);

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

// github fields (shared with forgejo)
const ghOwner = ref("");
const ghRepo = ref("");
const ghRef = ref("");
const ghAssetId = ref("");
const ghFilename = ref("");
const ghFilePath = ref("");

// nuget fields
const nugetPackage = ref("");
const nugetVersion = ref("");

// pypi fields
const pypiPackage = ref("");
const pypiVersion = ref("");
const pypiFilename = ref("");

// go fields
const goModule = ref("");
const goVersion = ref("");

// maven fields
const mavenGroup = ref("");
const mavenArtifact = ref("");
const mavenVersion = ref("");
const mavenFilename = ref("");

// terraform fields
const tfNamespace = ref("");
const tfName = ref("");
const tfProvider = ref("");
const tfVersion = ref("");
const tfOs = ref("");
const tfArch = ref("");

// rubygems fields
const gemName = ref("");
const gemVersion = ref("");

// conda fields
const condaPlatform = ref("linux-64");
const condaFilename = ref("");

// openvsx fields
const vsxPublisher = ref("");
const vsxName = ref("");
const vsxVersion = ref("");

// gitlab fields
const glProject = ref("");
const glTag = ref("");
const glLinkName = ref("");
const glFilePath = ref("");

// deb / rpm fields
const debPath = ref("");
const rpmPath = ref("");

// pacman fields
const pacmanArch = ref("x86_64");
const pacmanFilename = ref("");

// jetbrains fields
const jetbrainsPath = ref("");

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

function parseNpmUrl(parts: string[]): void {
  if (parts[0]) npmPackage.value = decodeURIComponent(parts[0]);
  if (parts[1] && parts[1] !== "-") {
    npmVersion.value = parts[1];
  } else if (parts[1] === "-" && parts[2]) {
    const m = parts[2].match(/-(\d[\w.\-+]*)\.tgz$/);
    if (m) npmVersion.value = m[1];
  }
}

function parseCargoUrl(parts: string[]): void {
  const idx = parts.indexOf("crates");
  if (idx >= 0) {
    cargoName.value = parts[idx + 1] ?? "";
    const maybeVer = parts[idx + 2];
    cargoVersion.value = maybeVer && maybeVer !== "download" ? maybeVer : "";
  }
}

function parseGithubComUrl(parts: string[]): void {
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
}

function parseRawGithubUrl(parts: string[]): void {
  ghOwner.value = parts[0] ?? "";
  ghRepo.value = parts[1] ?? "";
  ghRef.value = parts[2] ?? "";
  ghFilePath.value = parts.slice(3).join("/");
}

function parseApiGithubUrl(parts: string[]): void {
  if (parts[0] !== "repos") return;
  ghOwner.value = parts[1] ?? "";
  ghRepo.value = parts[2] ?? "";
  if (parts[3] === "releases" && parts[4] === "tags") ghRef.value = parts[5] ?? "";
  if (parts[3] === "releases" && parts[4] === "assets") ghAssetId.value = parts[5] ?? "";
}

function parsePackagistUrl(parts: string[]): void {
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

function parsePypiUrl(parts: string[]): void {
  // pypi.org/project/{name} or pypi.org/project/{name}/{version}
  if (parts[0] === "project" && parts[1]) {
    pypiPackage.value = parts[1];
    if (parts[2]) pypiVersion.value = parts[2];
  }
}

function parsePythonhostedUrl(parts: string[]): void {
  // files.pythonhosted.org/packages/…/{filename}
  const filename = parts[parts.length - 1];
  if (filename) pypiFilename.value = filename;
}

function parseRubygemsUrl(parts: string[]): void {
  // rubygems.org/gems/{name} or rubygems.org/gems/{name}/versions/{version}
  if (parts[0] === "gems" && parts[1]) {
    gemName.value = parts[1];
    if (parts[2] === "versions" && parts[3]) gemVersion.value = parts[3];
  }
}

function parseNugetUrl(parts: string[]): void {
  // nuget.org/packages/{id}/{version}
  if (parts[0] === "packages" && parts[1]) {
    nugetPackage.value = parts[1];
    if (parts[2]) nugetVersion.value = parts[2];
  }
}

function parseMavenUrl(parts: string[]): void {
  // search.maven.org/artifact/{groupId}/{artifactId}/{version}
  if (parts[0] === "artifact" && parts[1] && parts[2]) {
    mavenGroup.value = parts[1];
    mavenArtifact.value = parts[2];
    if (parts[3]) mavenVersion.value = parts[3];
  }
}

function parseGitlabUrl(parts: string[]): void {
  // gitlab.com/{group}/{project}/-/releases/{tag}
  // gitlab.com/{group}/{project}/-/archive/{tag}/{file}
  const sepIdx = parts.indexOf("-");
  if (sepIdx < 1) return;
  glProject.value = parts.slice(0, sepIdx).join("/");
  const sub = parts.slice(sepIdx + 1);
  if (sub[0] === "releases" && sub[1] === "tags" && sub[2]) glTag.value = sub[2];
  else if (sub[0] === "releases" && sub[1]) glTag.value = sub[1];
  else if (sub[0] === "archive" && sub[1]) glTag.value = sub[1];
}

function parseUrl(raw: string): void {
  const str = raw.trim();
  if (!str) return;
  try {
    const u = new URL(str);
    const parts = u.pathname.split("/").filter(Boolean);
    if (isHostOf(u.hostname, "npmjs.org") || isHostOf(u.hostname, "npmjs.com")) {
      registry.value = "npm";
      parseNpmUrl(parts);
    } else if (isHostOf(u.hostname, "crates.io")) {
      registry.value = "cargo";
      parseCargoUrl(parts);
    } else if (u.hostname === "github.com") {
      registry.value = "github";
      parseGithubComUrl(parts);
    } else if (u.hostname === "raw.githubusercontent.com") {
      registry.value = "github";
      parseRawGithubUrl(parts);
    } else if (u.hostname === "api.github.com") {
      registry.value = "github";
      parseApiGithubUrl(parts);
    } else if (u.hostname === "repo.packagist.org" || u.hostname === "packagist.org") {
      registry.value = "composer";
      parsePackagistUrl(parts);
    } else if (u.hostname === "pypi.org") {
      registry.value = "pypi";
      parsePypiUrl(parts);
    } else if (u.hostname === "files.pythonhosted.org") {
      registry.value = "pypi";
      parsePythonhostedUrl(parts);
    } else if (isHostOf(u.hostname, "rubygems.org")) {
      registry.value = "rubygems";
      parseRubygemsUrl(parts);
    } else if (isHostOf(u.hostname, "nuget.org")) {
      registry.value = "nuget";
      parseNugetUrl(parts);
    } else if (u.hostname === "search.maven.org") {
      registry.value = "maven";
      parseMavenUrl(parts);
    } else if (u.hostname === "gitlab.com") {
      registry.value = "gitlab";
      parseGitlabUrl(parts);
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

const nugetPaths = computed<ProxyPath[]>(() => {
  const reg = nugetRegistryName.value.trim() || "nuget";
  const id = nugetPackage.value.trim();
  const ver = nugetVersion.value.trim();
  return [
    { label: "Service index", url: `/proxy/${reg}/nuget/v3/index.json`, available: true },
    { label: "Search", url: `/proxy/${reg}/nuget/v3/query`, available: true },
    {
      label: "Flat — versions list",
      url: `/proxy/${reg}/nuget/v3/flat/${id}/index.json`,
      available: !!id,
    },
    {
      label: "Flat — .nupkg download",
      url: `/proxy/${reg}/nuget/v3/flat/${id}/${ver}/${id}.${ver}.nupkg`,
      available: !!id && !!ver,
    },
    {
      label: "Registration index",
      url: `/proxy/${reg}/nuget/v3/registration5/${id}/index.json`,
      available: !!id,
    },
    {
      label: "Yank version (DELETE)",
      url: `/proxy/${reg}/nuget/v2/package/${id}/${ver}`,
      available: !!id && !!ver,
    },
  ];
});

const pypiPaths = computed<ProxyPath[]>(() => {
  const reg = pypiRegistryName.value.trim() || "pypi";
  const pkg = pypiPackage.value.trim();
  const file = pypiFilename.value.trim();
  return [
    { label: "Simple index (all packages)", url: `/proxy/${reg}/simple/`, available: true },
    {
      label: "Package page",
      url: `/proxy/${reg}/simple/${pkg}/`,
      available: !!pkg,
    },
    {
      label: "Package file download",
      url: `/proxy/${reg}/packages/${file}`,
      available: !!file,
    },
    { label: "Publish (POST, twine)", url: `/proxy/${reg}/legacy/`, available: true },
  ];
});

const goproxyPaths = computed<ProxyPath[]>(() => {
  const reg = goproxyRegistryName.value.trim() || "goproxy";
  const mod = goModule.value.trim();
  const ver = goVersion.value.trim();
  if (!mod) return [];
  return [
    { label: "Latest version", url: `/proxy/${reg}/${mod}@latest`, available: true },
    { label: "Version list", url: `/proxy/${reg}/${mod}@v/list`, available: true },
    {
      label: "Version info (.info)",
      url: `/proxy/${reg}/${mod}@v/${ver}.info`,
      available: !!ver,
    },
    {
      label: "go.mod file",
      url: `/proxy/${reg}/${mod}@v/${ver}.mod`,
      available: !!ver,
    },
    {
      label: "Module zip (.zip)",
      url: `/proxy/${reg}/${mod}@v/${ver}.zip`,
      available: !!ver,
    },
  ];
});

const mavenPaths = computed<ProxyPath[]>(() => {
  const reg = mavenRegistryName.value.trim() || "maven";
  const group = mavenGroup.value.trim();
  const artifact = mavenArtifact.value.trim();
  const ver = mavenVersion.value.trim();
  const file = mavenFilename.value.trim();
  // Maven groupId uses dots; the path uses slashes
  const groupPath = group.replace(/\./g, "/");
  const hasCoords = !!group && !!artifact;
  const defaultFile = hasCoords && ver ? `${artifact}-${ver}.jar` : "";
  return [
    {
      label: "Artifact directory",
      url: `/proxy/${reg}/maven2/${groupPath}/${artifact}/`,
      available: hasCoords,
    },
    {
      label: "Version directory",
      url: `/proxy/${reg}/maven2/${groupPath}/${artifact}/${ver}/`,
      available: hasCoords && !!ver,
    },
    {
      label: "File download",
      url: `/proxy/${reg}/maven2/${groupPath}/${artifact}/${ver}/${file || defaultFile}`,
      available: hasCoords && !!ver,
    },
    {
      label: "POM file",
      url: `/proxy/${reg}/maven2/${groupPath}/${artifact}/${ver}/${artifact}-${ver}.pom`,
      available: hasCoords && !!ver,
    },
  ];
});

const terraformPaths = computed<ProxyPath[]>(() => {
  const reg = terraformRegistryName.value.trim() || "terraform";
  const ns = tfNamespace.value.trim();
  const name = tfName.value.trim();
  const provider = tfProvider.value.trim();
  const ver = tfVersion.value.trim();
  const os = tfOs.value.trim();
  const arch = tfArch.value.trim();
  const hasModule = !!ns && !!name && !!provider;
  const hasProvider = !!ns && !!provider;
  return [
    {
      label: "Module versions",
      url: `/proxy/${reg}/v1/modules/${ns}/${name}/${provider}/versions`,
      available: hasModule,
    },
    {
      label: "Module download",
      url: `/proxy/${reg}/v1/modules/${ns}/${name}/${provider}/${ver}/download`,
      available: hasModule && !!ver,
    },
    {
      label: "Module artifact",
      url: `/proxy/${reg}/v1/modules/${ns}/${name}/${provider}/${ver}/artifact`,
      available: hasModule && !!ver,
    },
    {
      label: "Provider versions",
      url: `/proxy/${reg}/v1/providers/${ns}/${provider}/versions`,
      available: hasProvider,
    },
    {
      label: "Provider download",
      url: `/proxy/${reg}/v1/providers/${ns}/${provider}/${ver}/download/${os}/${arch}`,
      available: hasProvider && !!ver && !!os && !!arch,
    },
    {
      label: "Provider artifact",
      url: `/proxy/${reg}/v1/providers/${ns}/${provider}/${ver}/artifact/${os}/${arch}`,
      available: hasProvider && !!ver && !!os && !!arch,
    },
  ];
});

const rubygemsPaths = computed<ProxyPath[]>(() => {
  const reg = rubygemsRegistryName.value.trim() || "rubygems";
  const name = gemName.value.trim();
  const ver = gemVersion.value.trim();
  return [
    { label: "Full specs index", url: `/proxy/${reg}/specs.4.8.gz`, available: true },
    { label: "Latest specs index", url: `/proxy/${reg}/latest_specs.4.8.gz`, available: true },
    { label: "Prerelease specs", url: `/proxy/${reg}/prerelease_specs.4.8.gz`, available: true },
    {
      label: "Gem info (JSON)",
      url: `/proxy/${reg}/api/v1/gems/${name}.json`,
      available: !!name,
    },
    {
      label: "Version list (JSON)",
      url: `/proxy/${reg}/api/v1/versions/${name}.json`,
      available: !!name,
    },
    {
      label: "Gemspec",
      url: `/proxy/${reg}/quick/Marshal.4.8/${name}-${ver}.gemspec.rz`,
      available: !!name && !!ver,
    },
    {
      label: "Gem download",
      url: `/proxy/${reg}/gems/${name}-${ver}.gem`,
      available: !!name && !!ver,
    },
  ];
});

const condaPaths = computed<ProxyPath[]>(() => {
  const reg = condaRegistryName.value.trim() || "conda";
  const platform = condaPlatform.value.trim() || "linux-64";
  const file = condaFilename.value.trim();
  return [
    {
      label: "repodata.json",
      url: `/proxy/${reg}/${platform}/repodata.json`,
      available: true,
    },
    {
      label: "current_repodata.json",
      url: `/proxy/${reg}/${platform}/current_repodata.json`,
      available: true,
    },
    {
      label: "Package file",
      url: `/proxy/${reg}/${platform}/${file}`,
      available: !!file,
    },
  ];
});

const openvsxPaths = computed<ProxyPath[]>(() => {
  const reg = openvsxRegistryName.value.trim() || "openvsx";
  const pub = vsxPublisher.value.trim();
  const name = vsxName.value.trim();
  const ver = vsxVersion.value.trim();
  const extId = pub && name ? `${pub}.${name}` : "";
  return [
    {
      label: "VSIX download",
      url: `/proxy/${reg}/${extId}/${ver}/vsix`,
      available: !!extId && !!ver,
    },
  ];
});

const forgejoPaths = computed<ProxyPath[]>(() => {
  const reg = forgejoRegistryName.value.trim() || "forgejo";
  const owner = ghOwner.value.trim();
  const repo = ghRepo.value.trim();
  const ref = ghRef.value.trim();
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
      label: "Raw file",
      url: `/proxy/${reg}/${base}/raw/${ref}/${file}`,
      available: !!ref && !!file,
    },
    {
      label: "Package API passthrough",
      url: `/proxy/${reg}/api/packages/${owner}/<package-type>/<name>/<version>/<file>`,
      available: !!owner,
    },
  ];
});

const gitlabPaths = computed<ProxyPath[]>(() => {
  const reg = gitlabRegistryName.value.trim() || "gitlab";
  const project = glProject.value.trim();
  const tag = glTag.value.trim();
  const link = glLinkName.value.trim();
  const file = glFilePath.value.trim();
  if (!project) return [];
  return [
    {
      label: "List releases",
      url: `/proxy/${reg}/${project}/-/releases`,
      available: true,
    },
    {
      label: "Release by tag",
      url: `/proxy/${reg}/${project}/-/releases/${tag}`,
      available: !!tag,
    },
    {
      label: "Release link download",
      url: `/proxy/${reg}/${project}/-/releases/${tag}/downloads/${link}`,
      available: !!tag && !!link,
    },
    {
      label: "Source archive",
      url: `/proxy/${reg}/${project}/-/archive/${tag}/source.tar.gz`,
      available: !!tag,
    },
    {
      label: "Raw file",
      url: `/proxy/${reg}/${project}/-/raw/${tag}/${file}`,
      available: !!tag && !!file,
    },
    {
      label: "API v4 passthrough",
      url: `/proxy/${reg}/api/v4/<path>`,
      available: true,
    },
  ];
});

const debPaths = computed<ProxyPath[]>(() => {
  const reg = debRegistryName.value.trim() || "deb";
  const path = debPath.value.trim();
  return [
    {
      label: "Repository path",
      url: `/proxy/${reg}/deb/${path || "<suite>/<component>/<file>"}`,
      available: !!path,
    },
    {
      label: "Signing key",
      url: `/proxy/${reg}/deb/key.gpg`,
      available: true,
    },
  ];
});

const rpmPaths = computed<ProxyPath[]>(() => {
  const reg = rpmRegistryName.value.trim() || "rpm";
  const path = rpmPath.value.trim();
  return [
    {
      label: "Repository path",
      url: `/proxy/${reg}/rpm/${path || "<path>"}`,
      available: !!path,
    },
    {
      label: "Signing key",
      url: `/proxy/${reg}/rpm/repodata/repomd.xml.key`,
      available: true,
    },
  ];
});

const pacmanPaths = computed<ProxyPath[]>(() => {
  const reg = pacmanRegistryName.value.trim() || "pacman";
  const arch = pacmanArch.value.trim() || "x86_64";
  const file = pacmanFilename.value.trim();
  return [
    {
      label: "Package file",
      url: `/proxy/${reg}/pacman/${arch}/${file}`,
      available: !!file,
    },
    {
      label: "Repository DB",
      url: `/proxy/${reg}/pacman/${arch}/${reg}.db`,
      available: true,
    },
    {
      label: "Signing key",
      url: `/proxy/${reg}/pacman/key.gpg`,
      available: true,
    },
  ];
});

const jetbrainsPaths = computed<ProxyPath[]>(() => {
  const reg = jetbrainsRegistryName.value.trim() || "jetbrains";
  const path = jetbrainsPath.value.trim();
  return [
    {
      label: "File download",
      url: `/proxy/${reg}/jetbrains/${path || "<product>/<filename>"}`,
      available: !!path,
    },
  ];
});

const activePaths = computed(() => {
  if (registry.value === "npm") return npmPaths.value;
  if (registry.value === "cargo") return cargoPaths.value;
  if (registry.value === "composer") return composerPaths.value;
  if (registry.value === "nuget") return nugetPaths.value;
  if (registry.value === "pypi") return pypiPaths.value;
  if (registry.value === "goproxy") return goproxyPaths.value;
  if (registry.value === "maven") return mavenPaths.value;
  if (registry.value === "terraform") return terraformPaths.value;
  if (registry.value === "rubygems") return rubygemsPaths.value;
  if (registry.value === "conda") return condaPaths.value;
  if (registry.value === "openvsx") return openvsxPaths.value;
  if (registry.value === "forgejo") return forgejoPaths.value;
  if (registry.value === "gitlab") return gitlabPaths.value;
  if (registry.value === "deb") return debPaths.value;
  if (registry.value === "rpm") return rpmPaths.value;
  if (registry.value === "pacman") return pacmanPaths.value;
  if (registry.value === "jetbrains") return jetbrainsPaths.value;
  return githubPaths.value;
});

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
          placeholder="https://pypi.org/project/requests/… or https://github.com/owner/repo/…"
          class="mt-1.5 font-mono text-sm"
        />
      </CardContent>
    </Card>

    <!-- Registry selector -->
    <div class="space-y-1">
      <Label for="registry-select" class="text-xs uppercase tracking-wide text-muted-foreground">
        Registry type
      </Label>
      <select
        id="registry-select"
        v-model="registry"
        class="w-full rounded-sm border border-border bg-background px-3 py-2 font-mono text-sm focus:outline-none focus:ring-1 focus:ring-primary"
      >
        <optgroup label="Git hosting">
          <option value="github">GitHub</option>
          <option value="forgejo">Forgejo / Gitea</option>
          <option value="gitlab">GitLab</option>
        </optgroup>
        <optgroup label="Language registries">
          <option value="npm">npm (JavaScript)</option>
          <option value="cargo">Cargo (Rust)</option>
          <option value="nuget">NuGet (.NET)</option>
          <option value="pypi">PyPI (Python)</option>
          <option value="goproxy">Go modules</option>
          <option value="maven">Maven (Java)</option>
          <option value="rubygems">RubyGems</option>
          <option value="composer">Composer (PHP)</option>
        </optgroup>
        <optgroup label="DevOps / IDE">
          <option value="terraform">Terraform</option>
          <option value="openvsx">OpenVSX (VS Code extensions)</option>
          <option value="jetbrains">JetBrains IDE</option>
        </optgroup>
        <optgroup label="Scientific">
          <option value="conda">Conda</option>
        </optgroup>
        <optgroup label="Linux packages">
          <option value="deb">Debian APT</option>
          <option value="rpm">RPM (YUM/DNF)</option>
          <option value="pacman">Arch Linux (pacman)</option>
        </optgroup>
      </select>
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

    <!-- Forgejo / Gitea fields (shares owner/repo/ref inputs with GitHub) -->
    <div v-else-if="registry === 'forgejo'" class="space-y-4">
      <div class="space-y-1">
        <Label for="fj-registry">Registry name</Label>
        <Input
          id="fj-registry"
          v-model="forgejoRegistryName"
          list="pm-forgejo-list"
          placeholder="forgejo"
          class="font-mono"
        />
        <datalist id="pm-forgejo-list">
          <option v-for="r in forgejoRegistries" :key="r.name" :value="r.name" />
        </datalist>
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div class="space-y-1">
          <Label for="fj-owner">Owner</Label>
          <Input id="fj-owner" v-model="ghOwner" placeholder="myorg" />
        </div>
        <div class="space-y-1">
          <Label for="fj-repo">Repository</Label>
          <Input id="fj-repo" v-model="ghRepo" placeholder="myproject" />
        </div>
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div class="space-y-1">
          <Label for="fj-ref">Tag / branch / SHA</Label>
          <Input id="fj-ref" v-model="ghRef" placeholder="v1.0.0" />
        </div>
        <div class="space-y-1">
          <Label for="fj-filename"
            >Asset filename <span class="text-muted-foreground">(optional)</span></Label
          >
          <Input id="fj-filename" v-model="ghFilename" placeholder="app-linux-amd64.tar.gz" class="font-mono" />
        </div>
      </div>
      <div class="space-y-1">
        <Label for="fj-file"
          >Raw file path <span class="text-muted-foreground">(optional)</span></Label
        >
        <Input id="fj-file" v-model="ghFilePath" placeholder="README.md" class="font-mono" />
      </div>
    </div>

    <!-- GitLab fields -->
    <div v-else-if="registry === 'gitlab'" class="space-y-4">
      <div class="space-y-1">
        <Label for="gl-registry">Registry name</Label>
        <Input
          id="gl-registry"
          v-model="gitlabRegistryName"
          list="pm-gitlab-list"
          placeholder="gitlab"
          class="font-mono"
        />
        <datalist id="pm-gitlab-list">
          <option v-for="r in gitlabRegistries" :key="r.name" :value="r.name" />
        </datalist>
      </div>
      <div class="space-y-1">
        <Label for="gl-project">Project path</Label>
        <Input
          id="gl-project"
          v-model="glProject"
          placeholder="group/subgroup/project"
          class="font-mono"
        />
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div class="space-y-1">
          <Label for="gl-tag">Tag <span class="text-muted-foreground">(optional)</span></Label>
          <Input id="gl-tag" v-model="glTag" placeholder="v1.0.0" class="font-mono" />
        </div>
        <div class="space-y-1">
          <Label for="gl-link">Link name <span class="text-muted-foreground">(optional)</span></Label>
          <Input id="gl-link" v-model="glLinkName" placeholder="app.bin" class="font-mono" />
        </div>
      </div>
      <div class="space-y-1">
        <Label for="gl-file">File path <span class="text-muted-foreground">(optional, for raw)</span></Label>
        <Input id="gl-file" v-model="glFilePath" placeholder="README.md" class="font-mono" />
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

    <!-- NuGet fields -->
    <div v-else-if="registry === 'nuget'" class="space-y-4">
      <div class="space-y-1">
        <Label for="nuget-registry">Registry name</Label>
        <Input
          id="nuget-registry"
          v-model="nugetRegistryName"
          list="pm-nuget-list"
          placeholder="nuget"
          class="font-mono"
        />
        <datalist id="pm-nuget-list">
          <option v-for="r in nugetRegistries" :key="r.name" :value="r.name" />
        </datalist>
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div class="space-y-1">
          <Label for="nuget-pkg">Package ID</Label>
          <Input id="nuget-pkg" v-model="nugetPackage" placeholder="Newtonsoft.Json" class="font-mono" />
        </div>
        <div class="space-y-1">
          <Label for="nuget-ver">Version <span class="text-muted-foreground">(optional)</span></Label>
          <Input id="nuget-ver" v-model="nugetVersion" placeholder="13.0.3" class="font-mono" />
        </div>
      </div>
    </div>

    <!-- PyPI fields -->
    <div v-else-if="registry === 'pypi'" class="space-y-4">
      <div class="space-y-1">
        <Label for="pypi-registry">Registry name</Label>
        <Input
          id="pypi-registry"
          v-model="pypiRegistryName"
          list="pm-pypi-list"
          placeholder="pypi"
          class="font-mono"
        />
        <datalist id="pm-pypi-list">
          <option v-for="r in pypiRegistries" :key="r.name" :value="r.name" />
        </datalist>
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div class="space-y-1">
          <Label for="pypi-pkg">Package <span class="text-muted-foreground">(optional)</span></Label>
          <Input id="pypi-pkg" v-model="pypiPackage" placeholder="requests" class="font-mono" />
        </div>
        <div class="space-y-1">
          <Label for="pypi-ver">Version <span class="text-muted-foreground">(optional)</span></Label>
          <Input id="pypi-ver" v-model="pypiVersion" placeholder="2.31.0" class="font-mono" />
        </div>
      </div>
      <div class="space-y-1">
        <Label for="pypi-file">Filename <span class="text-muted-foreground">(optional — for direct file download)</span></Label>
        <Input
          id="pypi-file"
          v-model="pypiFilename"
          placeholder="requests-2.31.0-py3-none-any.whl"
          class="font-mono"
        />
      </div>
    </div>

    <!-- Go module fields -->
    <div v-else-if="registry === 'goproxy'" class="space-y-4">
      <div class="space-y-1">
        <Label for="go-registry">Registry name</Label>
        <Input
          id="go-registry"
          v-model="goproxyRegistryName"
          list="pm-go-list"
          placeholder="goproxy"
          class="font-mono"
        />
        <datalist id="pm-go-list">
          <option v-for="r in goproxyRegistries" :key="r.name" :value="r.name" />
        </datalist>
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div class="space-y-1">
          <Label for="go-module">Module path</Label>
          <Input id="go-module" v-model="goModule" placeholder="github.com/gin-gonic/gin" class="font-mono" />
        </div>
        <div class="space-y-1">
          <Label for="go-ver">Version <span class="text-muted-foreground">(optional)</span></Label>
          <Input id="go-ver" v-model="goVersion" placeholder="v1.9.1" class="font-mono" />
        </div>
      </div>
    </div>

    <!-- Maven fields -->
    <div v-else-if="registry === 'maven'" class="space-y-4">
      <div class="space-y-1">
        <Label for="maven-registry">Registry name</Label>
        <Input
          id="maven-registry"
          v-model="mavenRegistryName"
          list="pm-maven-list"
          placeholder="maven"
          class="font-mono"
        />
        <datalist id="pm-maven-list">
          <option v-for="r in mavenRegistries" :key="r.name" :value="r.name" />
        </datalist>
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div class="space-y-1">
          <Label for="maven-group">Group ID</Label>
          <Input id="maven-group" v-model="mavenGroup" placeholder="com.google.guava" class="font-mono" />
        </div>
        <div class="space-y-1">
          <Label for="maven-artifact">Artifact ID</Label>
          <Input id="maven-artifact" v-model="mavenArtifact" placeholder="guava" class="font-mono" />
        </div>
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div class="space-y-1">
          <Label for="maven-ver">Version <span class="text-muted-foreground">(optional)</span></Label>
          <Input id="maven-ver" v-model="mavenVersion" placeholder="32.1.3-jre" class="font-mono" />
        </div>
        <div class="space-y-1">
          <Label for="maven-file">Filename <span class="text-muted-foreground">(optional)</span></Label>
          <Input id="maven-file" v-model="mavenFilename" placeholder="guava-32.1.3-jre.jar" class="font-mono" />
        </div>
      </div>
      <p class="text-xs text-muted-foreground">
        Group IDs use dots (e.g. <code class="font-mono bg-muted px-1 rounded">com.google.guava</code>);
        the proxy converts them to slashes in the path automatically.
      </p>
    </div>

    <!-- Terraform fields -->
    <div v-else-if="registry === 'terraform'" class="space-y-4">
      <div class="space-y-1">
        <Label for="tf-registry">Registry name</Label>
        <Input
          id="tf-registry"
          v-model="terraformRegistryName"
          list="pm-tf-list"
          placeholder="terraform"
          class="font-mono"
        />
        <datalist id="pm-tf-list">
          <option v-for="r in terraformRegistries" :key="r.name" :value="r.name" />
        </datalist>
      </div>
      <div class="grid grid-cols-3 gap-3">
        <div class="space-y-1">
          <Label for="tf-ns">Namespace</Label>
          <Input id="tf-ns" v-model="tfNamespace" placeholder="hashicorp" class="font-mono" />
        </div>
        <div class="space-y-1">
          <Label for="tf-name">Name <span class="text-muted-foreground">(module)</span></Label>
          <Input id="tf-name" v-model="tfName" placeholder="consul" class="font-mono" />
        </div>
        <div class="space-y-1">
          <Label for="tf-provider">Provider / type</Label>
          <Input id="tf-provider" v-model="tfProvider" placeholder="aws" class="font-mono" />
        </div>
      </div>
      <div class="grid grid-cols-3 gap-3">
        <div class="space-y-1">
          <Label for="tf-ver">Version <span class="text-muted-foreground">(optional)</span></Label>
          <Input id="tf-ver" v-model="tfVersion" placeholder="1.0.0" class="font-mono" />
        </div>
        <div class="space-y-1">
          <Label for="tf-os">OS <span class="text-muted-foreground">(provider)</span></Label>
          <Input id="tf-os" v-model="tfOs" placeholder="linux" class="font-mono" />
        </div>
        <div class="space-y-1">
          <Label for="tf-arch">Arch <span class="text-muted-foreground">(provider)</span></Label>
          <Input id="tf-arch" v-model="tfArch" placeholder="amd64" class="font-mono" />
        </div>
      </div>
      <p class="text-xs text-muted-foreground">
        Module paths use <code class="font-mono bg-muted px-1 rounded">namespace/name/provider</code>.
        Provider paths use <code class="font-mono bg-muted px-1 rounded">namespace/type</code> — leave <em>Name</em> empty for provider-only paths.
      </p>
    </div>

    <!-- RubyGems fields -->
    <div v-else-if="registry === 'rubygems'" class="space-y-4">
      <div class="space-y-1">
        <Label for="rg-registry">Registry name</Label>
        <Input
          id="rg-registry"
          v-model="rubygemsRegistryName"
          list="pm-rg-list"
          placeholder="rubygems"
          class="font-mono"
        />
        <datalist id="pm-rg-list">
          <option v-for="r in rubygemsRegistries" :key="r.name" :value="r.name" />
        </datalist>
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div class="space-y-1">
          <Label for="gem-name">Gem name <span class="text-muted-foreground">(optional)</span></Label>
          <Input id="gem-name" v-model="gemName" placeholder="rails" class="font-mono" />
        </div>
        <div class="space-y-1">
          <Label for="gem-ver">Version <span class="text-muted-foreground">(optional)</span></Label>
          <Input id="gem-ver" v-model="gemVersion" placeholder="7.1.3" class="font-mono" />
        </div>
      </div>
    </div>

    <!-- Composer fields -->
    <div v-else-if="registry === 'composer'" class="space-y-4">
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

    <!-- Conda fields -->
    <div v-else-if="registry === 'conda'" class="space-y-4">
      <div class="space-y-1">
        <Label for="conda-registry">Registry name</Label>
        <Input
          id="conda-registry"
          v-model="condaRegistryName"
          list="pm-conda-list"
          placeholder="conda"
          class="font-mono"
        />
        <datalist id="pm-conda-list">
          <option v-for="r in condaRegistries" :key="r.name" :value="r.name" />
        </datalist>
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div class="space-y-1">
          <Label for="conda-platform">Platform</Label>
          <Input id="conda-platform" v-model="condaPlatform" placeholder="linux-64" class="font-mono" />
        </div>
        <div class="space-y-1">
          <Label for="conda-file">Package filename <span class="text-muted-foreground">(optional)</span></Label>
          <Input id="conda-file" v-model="condaFilename" placeholder="numpy-1.26.0-py311h0_0.conda" class="font-mono" />
        </div>
      </div>
    </div>

    <!-- OpenVSX fields -->
    <div v-else-if="registry === 'openvsx'" class="space-y-4">
      <div class="space-y-1">
        <Label for="vsx-registry">Registry name</Label>
        <Input
          id="vsx-registry"
          v-model="openvsxRegistryName"
          list="pm-vsx-list"
          placeholder="openvsx"
          class="font-mono"
        />
        <datalist id="pm-vsx-list">
          <option v-for="r in openvsxRegistries" :key="r.name" :value="r.name" />
        </datalist>
      </div>
      <div class="grid grid-cols-3 gap-3">
        <div class="space-y-1">
          <Label for="vsx-pub">Publisher</Label>
          <Input id="vsx-pub" v-model="vsxPublisher" placeholder="ms-python" class="font-mono" />
        </div>
        <div class="space-y-1">
          <Label for="vsx-name">Extension</Label>
          <Input id="vsx-name" v-model="vsxName" placeholder="python" class="font-mono" />
        </div>
        <div class="space-y-1">
          <Label for="vsx-ver">Version</Label>
          <Input id="vsx-ver" v-model="vsxVersion" placeholder="2024.0.0" class="font-mono" />
        </div>
      </div>
      <p class="text-xs text-muted-foreground">
        Extension IDs use the <code class="font-mono bg-muted px-1 rounded">publisher.name</code> convention.
      </p>
    </div>

    <!-- Debian APT fields -->
    <div v-else-if="registry === 'deb'" class="space-y-4">
      <div class="space-y-1">
        <Label for="deb-registry">Registry name</Label>
        <Input
          id="deb-registry"
          v-model="debRegistryName"
          list="pm-deb-list"
          placeholder="deb"
          class="font-mono"
        />
        <datalist id="pm-deb-list">
          <option v-for="r in debRegistries" :key="r.name" :value="r.name" />
        </datalist>
      </div>
      <div class="space-y-1">
        <Label for="deb-path">Path <span class="text-muted-foreground">(optional — e.g. stable/main/Packages.gz)</span></Label>
        <Input id="deb-path" v-model="debPath" placeholder="stable/main/Packages.gz" class="font-mono" />
      </div>
    </div>

    <!-- RPM fields -->
    <div v-else-if="registry === 'rpm'" class="space-y-4">
      <div class="space-y-1">
        <Label for="rpm-registry">Registry name</Label>
        <Input
          id="rpm-registry"
          v-model="rpmRegistryName"
          list="pm-rpm-list"
          placeholder="rpm"
          class="font-mono"
        />
        <datalist id="pm-rpm-list">
          <option v-for="r in rpmRegistries" :key="r.name" :value="r.name" />
        </datalist>
      </div>
      <div class="space-y-1">
        <Label for="rpm-path">Path <span class="text-muted-foreground">(optional — e.g. repodata/repomd.xml)</span></Label>
        <Input id="rpm-path" v-model="rpmPath" placeholder="repodata/repomd.xml" class="font-mono" />
      </div>
    </div>

    <!-- Arch Linux / pacman fields -->
    <div v-else-if="registry === 'pacman'" class="space-y-4">
      <div class="space-y-1">
        <Label for="pacman-registry">Registry name</Label>
        <Input
          id="pacman-registry"
          v-model="pacmanRegistryName"
          list="pm-pacman-list"
          placeholder="pacman"
          class="font-mono"
        />
        <datalist id="pm-pacman-list">
          <option v-for="r in pacmanRegistries" :key="r.name" :value="r.name" />
        </datalist>
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div class="space-y-1">
          <Label for="pacman-arch">Architecture</Label>
          <Input id="pacman-arch" v-model="pacmanArch" placeholder="x86_64" class="font-mono" />
        </div>
        <div class="space-y-1">
          <Label for="pacman-file">Package filename <span class="text-muted-foreground">(optional)</span></Label>
          <Input id="pacman-file" v-model="pacmanFilename" placeholder="hello-1.0-1-x86_64.pkg.tar.zst" class="font-mono" />
        </div>
      </div>
    </div>

    <!-- JetBrains fields -->
    <div v-else-if="registry === 'jetbrains'" class="space-y-4">
      <div class="space-y-1">
        <Label for="jb-registry">Registry name</Label>
        <Input
          id="jb-registry"
          v-model="jetbrainsRegistryName"
          list="pm-jb-list"
          placeholder="jetbrains"
          class="font-mono"
        />
        <datalist id="pm-jb-list">
          <option v-for="r in jetbrainsRegistries" :key="r.name" :value="r.name" />
        </datalist>
      </div>
      <div class="space-y-1">
        <Label for="jb-path">Archive path <span class="text-muted-foreground">(mirrors download.jetbrains.com path)</span></Label>
        <Input id="jb-path" v-model="jetbrainsPath" placeholder="idea/ideaIC-2024.1.4.tar.gz" class="font-mono" />
      </div>
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
          <span class="w-44 shrink-0 text-xs text-muted-foreground">{{ entry.label }}</span>
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
