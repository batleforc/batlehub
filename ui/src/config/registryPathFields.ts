export interface ProxyPath {
  label: string;
  url: string;
  available: boolean;
}

export interface PathFieldDef {
  /** Key into the per-type values object; also used for element id suffixes. */
  key: string;
  label: string;
  placeholder?: string;
  /** Muted-foreground text shown after the label, e.g. "(optional)" or "(module)". */
  suffix?: string;
  /** Apply the `font-mono` input class. */
  mono: boolean;
  /** Initial value; defaults to "". */
  default?: string;
  /** Fields sharing the same `row` number render together in one grid row. */
  row?: number;
}

export interface UrlParser {
  matchesHost: (hostname: string) => boolean;
  /** Returns the field values (by key) parsed out of the pasted URL's path segments. */
  parse: (parts: string[]) => Record<string, string>;
}

export interface RegistryPathTypeDef {
  id: string;
  label: string;
  /** `<optgroup>` label in the registry-type dropdown. */
  group: string;
  fields: PathFieldDef[];
  buildPaths: (registryName: string, values: Record<string, string>) => ProxyPath[];
  urlParsers?: UrlParser[];
  /** Trusted internal HTML shown below the field group (may contain `<code>`/`<em>`). */
  note?: string;
}

/**
 * Whether `hostname` is `domain` itself or a proper subdomain of it.
 * Unlike `hostname.includes(domain)`, this rejects look-alike hosts such as
 * `evil-npmjs.org.attacker.com` or `notnpmjs.org`.
 */
function isHostOf(hostname: string, domain: string): boolean {
  return hostname === domain || hostname.endsWith(`.${domain}`);
}

function v(values: Record<string, string>, key: string): string {
  return (values[key] ?? "").trim();
}

const OPTIONAL = "(optional)";

export const REGISTRY_PATH_TYPES: RegistryPathTypeDef[] = [
  // ── Git hosting ────────────────────────────────────────────────────────────
  {
    id: "github",
    label: "GitHub",
    group: "Git hosting",
    fields: [
      { key: "owner", label: "Owner", placeholder: "batleforc", mono: false, row: 1 },
      { key: "repo", label: "Repository", placeholder: "ProxyAuthK8S", mono: false, row: 1 },
      { key: "ref", label: "Tag / branch / SHA", placeholder: "v0.1.9", mono: false, row: 2 },
      {
        key: "assetId",
        label: "Asset ID",
        suffix: OPTIONAL,
        placeholder: "123456789",
        mono: false,
        row: 2,
      },
      {
        key: "filename",
        label: "Asset filename",
        suffix: OPTIONAL,
        placeholder: "tool-linux-amd64.tar.gz",
        mono: true,
      },
      {
        key: "filePath",
        label: "Raw file path",
        suffix: OPTIONAL,
        placeholder: "README.md or path/to/file.yaml",
        mono: true,
      },
    ],
    buildPaths: (reg, values) => {
      const owner = v(values, "owner");
      const repo = v(values, "repo");
      const ref = v(values, "ref");
      const asset = v(values, "assetId");
      const filename = v(values, "filename");
      const file = v(values, "filePath");
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
    },
    urlParsers: [
      {
        matchesHost: (h) => h === "github.com",
        parse: (parts) => {
          const out: Record<string, string> = { owner: parts[0] ?? "", repo: parts[1] ?? "" };
          if (parts[2] === "releases") {
            if (parts[3] === "tag" && parts[4]) out.ref = parts[4];
            if (parts[3] === "download" && parts[4]) {
              out.ref = parts[4];
              out.filename = parts[5] ?? "";
            }
          } else if (parts[2] === "archive") {
            const last = parts[parts.length - 1];
            out.ref = last.replace(/\.(tar\.gz|zip)$/, "").replace(/^refs\/tags\//, "");
          } else if (parts[2] === "blob" && parts[3]) {
            out.ref = parts[3];
            out.filePath = parts.slice(4).join("/");
          }
          return out;
        },
      },
      {
        matchesHost: (h) => h === "raw.githubusercontent.com",
        parse: (parts) => ({
          owner: parts[0] ?? "",
          repo: parts[1] ?? "",
          ref: parts[2] ?? "",
          filePath: parts.slice(3).join("/"),
        }),
      },
      {
        matchesHost: (h) => h === "api.github.com",
        parse: (parts) => {
          if (parts[0] !== "repos") return {};
          const out: Record<string, string> = { owner: parts[1] ?? "", repo: parts[2] ?? "" };
          if (parts[3] === "releases" && parts[4] === "tags") out.ref = parts[5] ?? "";
          if (parts[3] === "releases" && parts[4] === "assets") out.assetId = parts[5] ?? "";
          return out;
        },
      },
    ],
  },
  {
    id: "forgejo",
    label: "Forgejo / Gitea",
    group: "Git hosting",
    fields: [
      { key: "owner", label: "Owner", placeholder: "myorg", mono: false, row: 1 },
      { key: "repo", label: "Repository", placeholder: "myproject", mono: false, row: 1 },
      { key: "ref", label: "Tag / branch / SHA", placeholder: "v1.0.0", mono: false, row: 2 },
      {
        key: "filename",
        label: "Asset filename",
        suffix: OPTIONAL,
        placeholder: "app-linux-amd64.tar.gz",
        mono: true,
        row: 2,
      },
      {
        key: "filePath",
        label: "Raw file path",
        suffix: OPTIONAL,
        placeholder: "README.md",
        mono: true,
      },
    ],
    buildPaths: (reg, values) => {
      const owner = v(values, "owner");
      const repo = v(values, "repo");
      const ref = v(values, "ref");
      const filename = v(values, "filename");
      const file = v(values, "filePath");
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
    },
  },
  {
    id: "gitlab",
    label: "GitLab",
    group: "Git hosting",
    fields: [
      { key: "project", label: "Project path", placeholder: "group/subgroup/project", mono: true },
      { key: "tag", label: "Tag", suffix: OPTIONAL, placeholder: "v1.0.0", mono: true, row: 1 },
      {
        key: "linkName",
        label: "Link name",
        suffix: OPTIONAL,
        placeholder: "app.bin",
        mono: true,
        row: 1,
      },
      {
        key: "filePath",
        label: "File path",
        suffix: "(optional, for raw)",
        placeholder: "README.md",
        mono: true,
      },
    ],
    buildPaths: (reg, values) => {
      const project = v(values, "project");
      const tag = v(values, "tag");
      const link = v(values, "linkName");
      const file = v(values, "filePath");
      if (!project) return [];
      return [
        { label: "List releases", url: `/proxy/${reg}/${project}/-/releases`, available: true },
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
        { label: "API v4 passthrough", url: `/proxy/${reg}/api/v4/<path>`, available: true },
      ];
    },
    urlParsers: [
      {
        matchesHost: (h) => h === "gitlab.com",
        parse: (parts) => {
          const sepIdx = parts.indexOf("-");
          if (sepIdx < 1) return {};
          const out: Record<string, string> = { project: parts.slice(0, sepIdx).join("/") };
          const sub = parts.slice(sepIdx + 1);
          if (sub[0] === "releases" && sub[1] === "tags" && sub[2]) out.tag = sub[2];
          else if (sub[0] === "releases" && sub[1]) out.tag = sub[1];
          else if (sub[0] === "archive" && sub[1]) out.tag = sub[1];
          return out;
        },
      },
    ],
  },

  // ── Language registries ─────────────────────────────────────────────────────
  {
    id: "npm",
    label: "npm (JavaScript)",
    group: "Language registries",
    fields: [
      { key: "package", label: "Package", placeholder: "lodash", mono: true, row: 1 },
      {
        key: "version",
        label: "Version",
        suffix: OPTIONAL,
        placeholder: "4.17.21",
        mono: true,
        row: 1,
      },
    ],
    buildPaths: (reg, values) => {
      const pkg = v(values, "package");
      const ver = v(values, "version");
      if (!pkg) return [];
      return [
        { label: "Packument (all versions)", url: `/proxy/${reg}/${pkg}`, available: true },
        { label: "Version metadata", url: `/proxy/${reg}/${pkg}/${ver}`, available: !!ver },
        { label: "Tarball download", url: `/proxy/${reg}/${pkg}/${ver}/tarball`, available: !!ver },
      ];
    },
    urlParsers: [
      {
        matchesHost: (h) => isHostOf(h, "npmjs.org") || isHostOf(h, "npmjs.com"),
        parse: (parts) => {
          const out: Record<string, string> = {};
          if (parts[0]) out.package = decodeURIComponent(parts[0]);
          if (parts[1] && parts[1] !== "-") {
            out.version = parts[1];
          } else if (parts[1] === "-" && parts[2]) {
            const m = parts[2].match(/-(\d[\w.\-+]*)\.tgz$/);
            if (m) out.version = m[1];
          }
          return out;
        },
      },
    ],
  },
  {
    id: "cargo",
    label: "Cargo (Rust)",
    group: "Language registries",
    fields: [
      { key: "name", label: "Crate", placeholder: "serde", mono: true, row: 1 },
      {
        key: "version",
        label: "Version",
        suffix: OPTIONAL,
        placeholder: "1.0.197",
        mono: true,
        row: 1,
      },
    ],
    buildPaths: (reg, values) => {
      const name = v(values, "name");
      const ver = v(values, "version");
      if (!name) return [];
      return [
        { label: "Crate metadata (all versions)", url: `/proxy/${reg}/${name}`, available: true },
        { label: "Version metadata", url: `/proxy/${reg}/${name}/${ver}`, available: !!ver },
        {
          label: ".crate download",
          url: `/proxy/${reg}/${name}/${ver}/download`,
          available: !!ver,
        },
        {
          label: "Sparse index config",
          url: `/proxy/${reg}/registry/config.json`,
          available: true,
        },
      ];
    },
    urlParsers: [
      {
        matchesHost: (h) => isHostOf(h, "crates.io"),
        parse: (parts) => {
          const idx = parts.indexOf("crates");
          if (idx < 0) return {};
          const out: Record<string, string> = { name: parts[idx + 1] ?? "" };
          const maybeVer = parts[idx + 2];
          out.version = maybeVer && maybeVer !== "download" ? maybeVer : "";
          return out;
        },
      },
    ],
  },
  {
    id: "nuget",
    label: "NuGet (.NET)",
    group: "Language registries",
    fields: [
      { key: "package", label: "Package ID", placeholder: "Newtonsoft.Json", mono: true, row: 1 },
      {
        key: "version",
        label: "Version",
        suffix: OPTIONAL,
        placeholder: "13.0.3",
        mono: true,
        row: 1,
      },
    ],
    buildPaths: (reg, values) => {
      const id = v(values, "package");
      const ver = v(values, "version");
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
    },
    urlParsers: [
      {
        matchesHost: (h) => isHostOf(h, "nuget.org"),
        parse: (parts) => {
          if (parts[0] !== "packages" || !parts[1]) return {};
          const out: Record<string, string> = { package: parts[1] };
          if (parts[2]) out.version = parts[2];
          return out;
        },
      },
    ],
  },
  {
    id: "pypi",
    label: "PyPI (Python)",
    group: "Language registries",
    fields: [
      {
        key: "package",
        label: "Package",
        suffix: OPTIONAL,
        placeholder: "requests",
        mono: true,
        row: 1,
      },
      {
        key: "version",
        label: "Version",
        suffix: OPTIONAL,
        placeholder: "2.31.0",
        mono: true,
        row: 1,
      },
      {
        key: "filename",
        label: "Filename",
        suffix: "(optional — for direct file download)",
        placeholder: "requests-2.31.0-py3-none-any.whl",
        mono: true,
      },
    ],
    buildPaths: (reg, values) => {
      const pkg = v(values, "package");
      const file = v(values, "filename");
      return [
        { label: "Simple index (all packages)", url: `/proxy/${reg}/simple/`, available: true },
        { label: "Package page", url: `/proxy/${reg}/simple/${pkg}/`, available: !!pkg },
        {
          label: "Package file download",
          url: `/proxy/${reg}/packages/${file}`,
          available: !!file,
        },
        { label: "Publish (POST, twine)", url: `/proxy/${reg}/legacy/`, available: true },
      ];
    },
    urlParsers: [
      {
        matchesHost: (h) => h === "pypi.org",
        parse: (parts) => {
          if (parts[0] !== "project" || !parts[1]) return {};
          const out: Record<string, string> = { package: parts[1] };
          if (parts[2]) out.version = parts[2];
          return out;
        },
      },
      {
        matchesHost: (h) => h === "files.pythonhosted.org",
        parse: (parts): Record<string, string> => {
          const filename = parts[parts.length - 1];
          return filename ? { filename } : {};
        },
      },
    ],
  },
  {
    id: "goproxy",
    label: "Go modules",
    group: "Language registries",
    fields: [
      {
        key: "module",
        label: "Module path",
        placeholder: "github.com/gin-gonic/gin",
        mono: true,
        row: 1,
      },
      {
        key: "version",
        label: "Version",
        suffix: OPTIONAL,
        placeholder: "v1.9.1",
        mono: true,
        row: 1,
      },
    ],
    buildPaths: (reg, values) => {
      const mod = v(values, "module");
      const ver = v(values, "version");
      if (!mod) return [];
      return [
        { label: "Latest version", url: `/proxy/${reg}/${mod}@latest`, available: true },
        { label: "Version list", url: `/proxy/${reg}/${mod}@v/list`, available: true },
        {
          label: "Version info (.info)",
          url: `/proxy/${reg}/${mod}@v/${ver}.info`,
          available: !!ver,
        },
        { label: "go.mod file", url: `/proxy/${reg}/${mod}@v/${ver}.mod`, available: !!ver },
        { label: "Module zip (.zip)", url: `/proxy/${reg}/${mod}@v/${ver}.zip`, available: !!ver },
      ];
    },
  },
  {
    id: "maven",
    label: "Maven (Java)",
    group: "Language registries",
    fields: [
      { key: "group", label: "Group ID", placeholder: "com.google.guava", mono: true, row: 1 },
      { key: "artifact", label: "Artifact ID", placeholder: "guava", mono: true, row: 1 },
      {
        key: "version",
        label: "Version",
        suffix: OPTIONAL,
        placeholder: "32.1.3-jre",
        mono: true,
        row: 2,
      },
      {
        key: "filename",
        label: "Filename",
        suffix: OPTIONAL,
        placeholder: "guava-32.1.3-jre.jar",
        mono: true,
        row: 2,
      },
    ],
    note: 'Group IDs use dots (e.g. <code class="font-mono bg-muted px-1 rounded">com.google.guava</code>); the proxy converts them to slashes in the path automatically.',
    buildPaths: (reg, values) => {
      const group = v(values, "group");
      const artifact = v(values, "artifact");
      const ver = v(values, "version");
      const file = v(values, "filename");
      const groupPath = group.replaceAll(".", "/");
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
    },
    urlParsers: [
      {
        matchesHost: (h) => h === "search.maven.org",
        parse: (parts) => {
          if (parts[0] !== "artifact" || !parts[1] || !parts[2]) return {};
          const out: Record<string, string> = { group: parts[1], artifact: parts[2] };
          if (parts[3]) out.version = parts[3];
          return out;
        },
      },
    ],
  },
  {
    id: "rubygems",
    label: "RubyGems",
    group: "Language registries",
    fields: [
      {
        key: "name",
        label: "Gem name",
        suffix: OPTIONAL,
        placeholder: "rails",
        mono: true,
        row: 1,
      },
      {
        key: "version",
        label: "Version",
        suffix: OPTIONAL,
        placeholder: "7.1.3",
        mono: true,
        row: 1,
      },
    ],
    buildPaths: (reg, values) => {
      const name = v(values, "name");
      const ver = v(values, "version");
      return [
        { label: "Full specs index", url: `/proxy/${reg}/specs.4.8.gz`, available: true },
        { label: "Latest specs index", url: `/proxy/${reg}/latest_specs.4.8.gz`, available: true },
        {
          label: "Prerelease specs",
          url: `/proxy/${reg}/prerelease_specs.4.8.gz`,
          available: true,
        },
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
    },
    urlParsers: [
      {
        matchesHost: (h) => isHostOf(h, "rubygems.org"),
        parse: (parts) => {
          if (parts[0] !== "gems" || !parts[1]) return {};
          const out: Record<string, string> = { name: parts[1] };
          if (parts[2] === "versions" && parts[3]) out.version = parts[3];
          return out;
        },
      },
    ],
  },
  {
    id: "composer",
    label: "Composer (PHP)",
    group: "Language registries",
    fields: [
      { key: "vendor", label: "Vendor", placeholder: "symfony", mono: true, row: 1 },
      { key: "package", label: "Package", placeholder: "console", mono: true, row: 1 },
      {
        key: "version",
        label: "Version",
        suffix: "(optional — for dist download and yank)",
        placeholder: "7.1.0",
        mono: true,
      },
    ],
    note: 'Package names follow the <code class="font-mono bg-muted px-1 rounded">vendor/package</code> convention. Paste a <code class="font-mono bg-muted px-1 rounded">packagist.org</code> or <code class="font-mono bg-muted px-1 rounded">repo.packagist.org</code> URL above to auto-fill.',
    buildPaths: (reg, values) => {
      const vendor = v(values, "vendor");
      const pkg = v(values, "package");
      const ver = v(values, "version");
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
    },
    urlParsers: [
      {
        matchesHost: (h) => h === "repo.packagist.org" || h === "packagist.org",
        parse: (parts): Record<string, string> => {
          if (parts[0] === "p2" && parts[1] && parts[2]) {
            return {
              vendor: parts[1],
              package: parts[2].replace(/\.json$/, "").replace(/~dev$/, ""),
            };
          }
          if (parts[0] === "packages" && parts[1] && parts[2]) {
            return { vendor: parts[1], package: parts[2] };
          }
          return {};
        },
      },
    ],
  },

  // ── DevOps / IDE ─────────────────────────────────────────────────────────────
  {
    id: "terraform",
    label: "Terraform",
    group: "DevOps / IDE",
    fields: [
      { key: "namespace", label: "Namespace", placeholder: "hashicorp", mono: true, row: 1 },
      { key: "name", label: "Name", suffix: "(module)", placeholder: "consul", mono: true, row: 1 },
      { key: "provider", label: "Provider / type", placeholder: "aws", mono: true, row: 1 },
      {
        key: "version",
        label: "Version",
        suffix: OPTIONAL,
        placeholder: "1.0.0",
        mono: true,
        row: 2,
      },
      { key: "os", label: "OS", suffix: "(provider)", placeholder: "linux", mono: true, row: 2 },
      {
        key: "arch",
        label: "Arch",
        suffix: "(provider)",
        placeholder: "amd64",
        mono: true,
        row: 2,
      },
    ],
    note: 'Module paths use <code class="font-mono bg-muted px-1 rounded">namespace/name/provider</code>. Provider paths use <code class="font-mono bg-muted px-1 rounded">namespace/type</code> — leave <em>Name</em> empty for provider-only paths.',
    buildPaths: (reg, values) => {
      const ns = v(values, "namespace");
      const name = v(values, "name");
      const provider = v(values, "provider");
      const ver = v(values, "version");
      const os = v(values, "os");
      const arch = v(values, "arch");
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
    },
  },
  {
    id: "openvsx",
    label: "OpenVSX (VS Code extensions)",
    group: "DevOps / IDE",
    fields: [
      { key: "publisher", label: "Publisher", placeholder: "ms-python", mono: true, row: 1 },
      { key: "name", label: "Extension", placeholder: "python", mono: true, row: 1 },
      { key: "version", label: "Version", placeholder: "2024.0.0", mono: true, row: 1 },
    ],
    note: 'Extension IDs use the <code class="font-mono bg-muted px-1 rounded">publisher.name</code> convention.',
    buildPaths: (reg, values) => {
      const pub = v(values, "publisher");
      const name = v(values, "name");
      const ver = v(values, "version");
      const extId = pub && name ? `${pub}.${name}` : "";
      return [
        {
          label: "VSIX download",
          url: `/proxy/${reg}/${extId}/${ver}/vsix`,
          available: !!extId && !!ver,
        },
      ];
    },
  },
  {
    id: "jetbrains",
    label: "JetBrains IDE",
    group: "DevOps / IDE",
    fields: [
      {
        key: "path",
        label: "Archive path",
        suffix: "(mirrors download.jetbrains.com path)",
        placeholder: "idea/ideaIC-2024.1.4.tar.gz",
        mono: true,
      },
    ],
    buildPaths: (reg, values) => {
      const path = v(values, "path");
      return [
        {
          label: "File download",
          url: `/proxy/${reg}/jetbrains/${path || "<product>/<filename>"}`,
          available: !!path,
        },
      ];
    },
  },

  // ── Scientific ───────────────────────────────────────────────────────────────
  {
    id: "conda",
    label: "Conda",
    group: "Scientific",
    fields: [
      {
        key: "platform",
        label: "Platform",
        placeholder: "linux-64",
        mono: true,
        default: "linux-64",
        row: 1,
      },
      {
        key: "filename",
        label: "Package filename",
        suffix: OPTIONAL,
        placeholder: "numpy-1.26.0-py311h0_0.conda",
        mono: true,
        row: 1,
      },
    ],
    buildPaths: (reg, values) => {
      const platform = v(values, "platform") || "linux-64";
      const file = v(values, "filename");
      return [
        { label: "repodata.json", url: `/proxy/${reg}/${platform}/repodata.json`, available: true },
        {
          label: "current_repodata.json",
          url: `/proxy/${reg}/${platform}/current_repodata.json`,
          available: true,
        },
        { label: "Package file", url: `/proxy/${reg}/${platform}/${file}`, available: !!file },
      ];
    },
  },

  // ── Linux packages ───────────────────────────────────────────────────────────
  {
    id: "deb",
    label: "Debian APT",
    group: "Linux packages",
    fields: [
      {
        key: "path",
        label: "Path",
        suffix: "(optional — e.g. stable/main/Packages.gz)",
        placeholder: "stable/main/Packages.gz",
        mono: true,
      },
    ],
    buildPaths: (reg, values) => {
      const path = v(values, "path");
      return [
        {
          label: "Repository path",
          url: `/proxy/${reg}/deb/${path || "<suite>/<component>/<file>"}`,
          available: !!path,
        },
        { label: "Signing key", url: `/proxy/${reg}/deb/key.gpg`, available: true },
      ];
    },
  },
  {
    id: "rpm",
    label: "RPM (YUM/DNF)",
    group: "Linux packages",
    fields: [
      {
        key: "path",
        label: "Path",
        suffix: "(optional — e.g. repodata/repomd.xml)",
        placeholder: "repodata/repomd.xml",
        mono: true,
      },
    ],
    buildPaths: (reg, values) => {
      const path = v(values, "path");
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
    },
  },
  {
    id: "pacman",
    label: "Arch Linux (pacman)",
    group: "Linux packages",
    fields: [
      {
        key: "arch",
        label: "Architecture",
        placeholder: "x86_64",
        mono: true,
        default: "x86_64",
        row: 1,
      },
      {
        key: "filename",
        label: "Package filename",
        suffix: OPTIONAL,
        placeholder: "hello-1.0-1-x86_64.pkg.tar.zst",
        mono: true,
        row: 1,
      },
    ],
    buildPaths: (reg, values) => {
      const arch = v(values, "arch") || "x86_64";
      const file = v(values, "filename");
      return [
        { label: "Package file", url: `/proxy/${reg}/pacman/${arch}/${file}`, available: !!file },
        { label: "Repository DB", url: `/proxy/${reg}/pacman/${arch}/${reg}.db`, available: true },
        { label: "Signing key", url: `/proxy/${reg}/pacman/key.gpg`, available: true },
      ];
    },
  },
];
