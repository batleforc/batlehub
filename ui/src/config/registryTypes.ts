import type { MeResponse } from "@/client/types.gen";

export interface SnippetContext {
  base: string;
  registryName: string;
  mode: string;
  isAuthenticated: boolean;
  token: string;
  netrcHost: string;
  netrcLogin: string;
  identity: MeResponse | null;
  /** All configured registries keyed by API type — used by composite tabs like mise. */
  selectedNames: Record<string, string>;
}

export interface SnippetDef {
  key: string;
  label?: string;
  lang: string;
  /** Trusted internal HTML displayed below the code block. */
  note?: string | ((ctx: SnippetContext) => string);
  template: (ctx: SnippetContext) => string;
  showWhen?: (ctx: SnippetContext) => boolean;
}

export interface RegistryTypeDef {
  id: string;
  label: string;
  fileHint?: string;
  /** Trusted internal HTML for the card description. */
  description: string;
  /** API `type` values that activate this tab. Defaults to `[id]`. */
  apiTypes?: string[];
  snippets: SnippetDef[];
}

const isPublishMode = (ctx: SnippetContext) => ctx.mode === "local" || ctx.mode === "hybrid";

/** Returns the user's token when authenticated, or a placeholder for unauthenticated previews. */
const authTokenOrPlaceholder = (ctx: SnippetContext) =>
  ctx.isAuthenticated ? ctx.token : "<your-token>";

/** Builds `registry=<url>` plus an optional `_authToken` line for npm-compatible `.npmrc` files. */
function buildNpmAuthLines(ctx: SnippetContext): string[] {
  const regUrl = `${ctx.base}/proxy/${ctx.registryName}/`;
  const lines = [`registry=${regUrl}`];
  if (ctx.isAuthenticated) {
    try {
      const { host, pathname } = new URL(regUrl);
      lines.push(`//${host}${pathname}:_authToken=${ctx.token}`);
    } catch {
      /* skip */
    }
  }
  return lines;
}

/** Embeds `netrcLogin`/`token` as HTTP Basic Auth credentials in `rawUrl`, when authenticated. */
function withCredentials(rawUrl: string, ctx: SnippetContext): string {
  if (!ctx.isAuthenticated) return rawUrl;
  try {
    const u = new URL(rawUrl);
    u.username = ctx.netrcLogin;
    u.password = ctx.token;
    return u.toString();
  } catch {
    return rawUrl;
  }
}

export const REGISTRY_TYPE_DEFS: RegistryTypeDef[] = [
  // ── mise (composite: github + npm + cargo) ─────────────────────────────────
  {
    id: "mise",
    label: "mise",
    fileHint: "mise.toml",
    description:
      `URL replacements intercept all HTTP requests made by mise (aqua, ubi, and other backends). ` +
      `Add to your global <code class="text-xs font-mono bg-muted px-1 rounded">~/.config/mise/config.toml</code> ` +
      `or a project-local <code class="text-xs font-mono bg-muted px-1 rounded">mise.toml</code>.`,
    apiTypes: ["github", "npm", "cargo"],
    snippets: [
      {
        key: "mise",
        lang: "toml",
        template: (ctx) => {
          const { base, isAuthenticated, token, netrcHost, netrcLogin, selectedNames } = ctx;
          const gh = selectedNames["github"];
          const np = selectedNames["npm"];
          const cg = selectedNames["cargo"];
          const lines: string[] = [];
          if (isAuthenticated) {
            lines.push(
              `# Authentication: mise reads ~/.netrc for HTTP Basic Auth`,
              `# machine ${netrcHost}`,
              `# login ${netrcLogin}`,
              `# password ${token}`,
              ``,
            );
          }
          lines.push(`[settings.url_replacements]`);
          if (gh) {
            lines.push(
              ``,
              `# ── GitHub (registry: ${gh}) ─────────────────────────────────────────────────`,
              `# API (release listings, tag metadata, asset lists)`,
              String.raw`"regex:^https://api\.github\.com/repos/(.+)" = "${base}/proxy/${gh}/$1"`,
              ``,
              `# Release asset binaries (browser_download_url from API responses)`,
              String.raw`"regex:^https://github\.com/([^/]+)/([^/]+)/releases/download/([^/]+)/(.+)" = "${base}/proxy/${gh}/$1/$2/releases/download/$3/$4"`,
              ``,
              `# Source tarballs`,
              String.raw`"regex:^https://github\.com/([^/]+)/([^/]+)/archive/(?:refs/tags/)?(.+?)\.tar\.gz" = "${base}/proxy/${gh}/$1/$2/tarball/$3"`,
              String.raw`"regex:^https://codeload\.github\.com/([^/]+)/([^/]+)/tar\.gz/(?:refs/tags/)?(.+)" = "${base}/proxy/${gh}/$1/$2/tarball/$3"`,
              ``,
              `# Zip archives`,
              String.raw`"regex:^https://github\.com/([^/]+)/([^/]+)/archive/(?:refs/tags/)?(.+?)\.zip" = "${base}/proxy/${gh}/$1/$2/zipball/$3"`,
              ``,
              `# Raw files (install scripts, manifests, …)`,
              String.raw`"regex:^https://raw\.githubusercontent\.com/([^/]+)/([^/]+)/([^/]+)/(.+)" = "${base}/proxy/${gh}/$1/$2/raw/$3/$4"`,
            );
          }
          if (np) {
            lines.push(
              ``,
              `# ── npm (registry: ${np}) ───────────────────────────────────────────────────`,
              String.raw`"regex:^https://registry\.npmjs\.org/(.+)" = "${base}/proxy/${np}/$1"`,
            );
          }
          if (cg) {
            lines.push(
              ``,
              `# ── Cargo (registry: ${cg}) — downloads only, use .cargo/config.toml for full support`,
              String.raw`"regex:^https://static\.crates\.io/crates/([^/]+)/([^/]+)/.+\.crate" = "${base}/proxy/${cg}/$1/$2/download"`,
            );
          }
          return lines.join("\n");
        },
      },
    ],
  },

  // ── npm ────────────────────────────────────────────────────────────────────
  {
    id: "npm",
    label: "npm",
    fileHint: ".npmrc",
    description:
      `Sets the registry for all packages. Place in your project root or ` +
      `<code class="text-xs font-mono bg-muted px-1 rounded">~/.npmrc</code> for global use.`,
    snippets: [
      {
        key: "npmrc",
        label: "npm / npm workspaces",
        lang: "ini",
        template: (ctx) => buildNpmAuthLines(ctx).join("\n"),
        note: (ctx) =>
          `To route only a specific scope through the proxy, use ` +
          `<code class="font-mono bg-muted px-1 rounded">@myorg:registry=${ctx.base}/proxy/${ctx.registryName}/</code> instead.`,
      },
      {
        key: "yarn",
        label: "Yarn Berry (.yarnrc.yml)",
        lang: "yaml",
        template: (ctx) => {
          const lines = [`npmRegistryServer: "${ctx.base}/proxy/${ctx.registryName}/"`];
          if (ctx.isAuthenticated) lines.push(`npmAuthToken: "${ctx.token}"`);
          return lines.join("\n");
        },
      },
      {
        key: "pnpm",
        label: "pnpm (.npmrc)",
        lang: "ini",
        template: (ctx) => buildNpmAuthLines(ctx).join("\n"),
      },
    ],
  },

  // ── Cargo ──────────────────────────────────────────────────────────────────
  {
    id: "cargo",
    label: "Cargo",
    fileHint: ".cargo/config.toml",
    description:
      `Replaces crates.io as the default source. Cargo fetches the sparse index and ` +
      `<code class="text-xs font-mono bg-muted px-1 rounded">.crate</code> files through the proxy. ` +
      `Add to your project's <code class="text-xs font-mono bg-muted px-1 rounded">.cargo/config.toml</code> ` +
      `or the global <code class="text-xs font-mono bg-muted px-1 rounded">~/.cargo/config.toml</code>.`,
    snippets: [
      {
        key: "cargo",
        lang: "toml",
        template: (ctx) => {
          const lines = [
            `[source.crates-io]`,
            `replace-with = "batlehub"`,
            ``,
            `[source.batlehub]`,
            `registry = "sparse+${ctx.base}/proxy/${ctx.registryName}/registry/"`,
          ];
          if (ctx.isAuthenticated) {
            lines.push(``, `[registries.batlehub]`, `token = "${ctx.token}"`);
          }
          return lines.join("\n");
        },
        note:
          `The proxy implements the ` +
          `<a href="https://doc.rust-lang.org/cargo/reference/registry-protocols.html#sparse-protocol" ` +
          `target="_blank" rel="noopener" class="underline underline-offset-2 hover:text-foreground transition-colors">` +
          `sparse registry protocol</a>. ` +
          `Checksums from the index match the cached <code class="font-mono bg-muted px-1 rounded">.crate</code> files, ` +
          `so <code class="font-mono bg-muted px-1 rounded">cargo verify-project</code> continues to work.`,
      },
    ],
  },

  // ── OpenVSX ────────────────────────────────────────────────────────────────
  {
    id: "openvsx",
    label: "OpenVSX",
    fileHint: "OpenVSX",
    description:
      `Proxy VS Code extension downloads from ` +
      `<a href="https://open-vsx.org" target="_blank" rel="noopener" ` +
      `class="underline underline-offset-2 hover:text-foreground transition-colors">open-vsx.org</a>. ` +
      `Extension IDs follow the <code class="text-xs font-mono bg-muted px-1 rounded">publisher.name</code> convention.`,
    snippets: [
      {
        key: "openvsx-direct",
        label: "Direct VSIX download URL",
        lang: "text",
        template: (ctx) =>
          `${ctx.base}/proxy/${ctx.registryName}/{publisher}.{extension}/{version}/vsix`,
        note:
          `Example: download and install via CLI — ` +
          `<code class="font-mono bg-muted px-1 rounded">` +
          `curl -L {proxy}/ms-python.python/2024.0.0/vsix -o ext.vsix &amp;&amp; code --install-extension ext.vsix` +
          `</code>`,
      },
      {
        key: "openvsx-mise",
        label: "mise — URL replacement to intercept VSIX downloads",
        lang: "toml",
        template: (ctx) => {
          const lines: string[] = [];
          if (ctx.isAuthenticated) {
            lines.push(
              `# Authentication: mise reads ~/.netrc for HTTP Basic Auth`,
              `# machine ${ctx.netrcHost}`,
              `# login ${ctx.netrcLogin}`,
              `# password ${ctx.token}`,
              ``,
            );
          }
          lines.push(
            `[settings.url_replacements]`,
            ``,
            `# ── OpenVSX VSIX downloads ────────────────────────────────────────────────────`,
            `# Intercepts VSIX file downloads from open-vsx.org and routes them through the proxy.`,
            `# The extension ID is joined as publisher.name to match the proxy convention.`,
            String.raw`"regex:^https://open-vsx\.org/api/([^/]+)/([^/]+)/([^/]+)/file/.+\.vsix$" = "${ctx.base}/proxy/${ctx.registryName}/$1.$2/$3/vsix"`,
          );
          return lines.join("\n");
        },
      },
      {
        key: "openvsx-vscodium",
        label: "VSCodium / Code - OSS extension gallery (product.json)",
        lang: "jsonc",
        template: (ctx) =>
          [
            `// ~/.config/VSCodium/User/product.json  (or merge into existing product.json)`,
            `{`,
            `  "extensionGallery": {`,
            `    "serviceUrl": "${ctx.base}/proxy/${ctx.registryName}/vscode/gallery",`,
            `    "itemUrl": "${ctx.base}/proxy/${ctx.registryName}/vscode/item",`,
            `    "resourceUrlTemplate": "${ctx.base}/proxy/${ctx.registryName}/vscode/unpkg/{publisher}/{name}/{version}/{path}"`,
            `  }`,
            `}`,
          ].join("\n"),
        note: (ctx) =>
          `Requires the proxy to implement the VS Code gallery protocol ` +
          `(<code class="font-mono bg-muted px-1 rounded">/vscode/gallery</code> endpoints). ` +
          `Only VSIX proxying is supported today.` +
          (ctx.isAuthenticated
            ? ` VSCodium does not support HTTP Basic Auth in ` +
              `<code class="font-mono bg-muted px-1 rounded">product.json</code>. ` +
              `Add your credentials to <code class="font-mono bg-muted px-1 rounded">~/.netrc</code> — see the <strong>.netrc</strong> tab.`
            : ""),
      },
    ],
  },

  // ── Go ─────────────────────────────────────────────────────────────────────
  {
    id: "goproxy",
    label: "Go",
    fileHint: "Go",
    description:
      `Set <code class="text-xs font-mono bg-muted px-1 rounded">GOPROXY</code> to route ` +
      `Go module downloads through this proxy. Modules are cached after the first download. ` +
      `Append <code class="text-xs font-mono bg-muted px-1 rounded">,direct</code> so the ` +
      `go tool falls back to the original source when the proxy returns 404.`,
    snippets: [
      {
        key: "go",
        label: "Environment variables",
        lang: "bash",
        template: (ctx) => {
          const proxyUrl = withCredentials(`${ctx.base}/proxy/${ctx.registryName}`, ctx);
          return [
            `# Shell / CI environment — set before running go commands`,
            `export GONOSUMCHECK="*"`,
            `export GONOSUMDB="*"`,
            `export GOPROXY="${proxyUrl},direct"`,
          ].join("\n");
        },
        note:
          `The proxy implements the ` +
          `<a href="https://go.dev/ref/mod#goproxy-protocol" target="_blank" rel="noopener" ` +
          `class="underline underline-offset-2 hover:text-foreground transition-colors">GOPROXY protocol</a>. ` +
          `Module zip archives are cached permanently after first download. ` +
          `<code class="font-mono bg-muted px-1 rounded">@latest</code> and ` +
          `<code class="font-mono bg-muted px-1 rounded">@v/list</code> responses are also cached — ` +
          `clear the proxy storage if you need to pick up newly published versions immediately.`,
      },
    ],
  },

  // ── Maven ──────────────────────────────────────────────────────────────────
  {
    id: "maven",
    label: "Maven",
    fileHint: "Maven",
    description:
      `Route Maven/Gradle dependency downloads through this proxy, or publish private artifacts ` +
      `(<code class="text-xs font-mono bg-muted px-1 rounded">mvn deploy</code>) when the registry ` +
      `is configured in <code class="text-xs font-mono bg-muted px-1 rounded">Local</code> ` +
      `or <code class="text-xs font-mono bg-muted px-1 rounded">Hybrid</code> mode.`,
    snippets: [
      {
        key: "maven-settings",
        label: "~/.m2/settings.xml — proxy all Maven dependencies",
        lang: "xml",
        template: (ctx) => {
          const { base, registryName: reg, isAuthenticated, token, netrcLogin } = ctx;
          const lines = [`<!-- ~/.m2/settings.xml -->`];
          if (isAuthenticated) {
            lines.push(
              `<settings>`,
              `  <servers>`,
              `    <server>`,
              `      <id>batlehub-${reg}</id>`,
              `      <username>${netrcLogin}</username>`,
              `      <password>${token}</password>`,
              `    </server>`,
              `  </servers>`,
              `  <mirrors>`,
              `    <mirror>`,
              `      <id>batlehub-${reg}</id>`,
              `      <name>BatleHub Maven Proxy</name>`,
              `      <url>${base}/proxy/${reg}/maven2/</url>`,
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
              `      <url>${base}/proxy/${reg}/maven2/</url>`,
              `      <mirrorOf>*</mirrorOf>`,
              `    </mirror>`,
              `  </mirrors>`,
              `</settings>`,
            );
          }
          return lines.join("\n");
        },
      },
      {
        key: "maven-publish",
        label: "pom.xml — publish private artifacts (Local / Hybrid mode)",
        lang: "xml",
        showWhen: isPublishMode,
        template: (ctx) => {
          const { base, registryName: reg } = ctx;
          return [
            `<!-- pom.xml — add <distributionManagement> inside <project> -->`,
            `<distributionManagement>`,
            `  <repository>`,
            `    <id>batlehub-${reg}</id>`,
            `    <url>${base}/proxy/${reg}/maven2/</url>`,
            `  </repository>`,
            `</distributionManagement>`,
            ``,
            `<!-- Then publish with: -->`,
            `<!-- mvn deploy -->`,
          ].join("\n");
        },
        note:
          `The registry must be configured with <code class="font-mono bg-muted px-1 rounded">mode = "local"</code> or ` +
          `<code class="font-mono bg-muted px-1 rounded">mode = "hybrid"</code> in ` +
          `<code class="font-mono bg-muted px-1 rounded">config.toml</code> to accept publishes. ` +
          `The <code class="font-mono bg-muted px-1 rounded">server</code> id in ` +
          `<code class="font-mono bg-muted px-1 rounded">settings.xml</code> must match the ` +
          `<code class="font-mono bg-muted px-1 rounded">repository id</code> in ` +
          `<code class="font-mono bg-muted px-1 rounded">distributionManagement</code>.`,
      },
    ],
  },

  // ── Terraform ──────────────────────────────────────────────────────────────
  {
    id: "terraform",
    label: "Terraform",
    fileHint: "Terraform",
    description:
      `Proxy Terraform provider downloads via network mirror, or publish private modules ` +
      `and providers when the registry is configured in ` +
      `<code class="text-xs font-mono bg-muted px-1 rounded">Local</code> ` +
      `or <code class="text-xs font-mono bg-muted px-1 rounded">Hybrid</code> mode.`,
    snippets: [
      {
        key: "terraformrc",
        label: "~/.terraformrc — provider network mirror",
        lang: "terraform",
        template: (ctx) => {
          const { base, registryName: reg, isAuthenticated, token } = ctx;
          let hostPart = base;
          try {
            hostPart = new URL(base).hostname;
          } catch {
            /* keep */
          }
          const lines = [
            `# ~/.terraformrc`,
            `provider_installation {`,
            `  network_mirror {`,
            `    url = "${base}/proxy/${reg}/"`,
            `  }`,
            `}`,
          ];
          if (isAuthenticated) {
            lines.push(``, `credentials "${hostPart}" {`, `  token = "${token}"`, `}`);
          }
          return lines.join("\n");
        },
        note:
          `The <code class="font-mono bg-muted px-1 rounded">network_mirror</code> block redirects all ` +
          `provider downloads through this proxy. Providers are cached after first download in ` +
          `Proxy/Hybrid mode, or served entirely locally in Local mode.`,
      },
      {
        key: "terraform-module",
        label: "Upload a private module (Local / Hybrid mode)",
        lang: "bash",
        showWhen: isPublishMode,
        template: (ctx) => {
          const { base, registryName: reg } = ctx;
          return [
            `# Upload a module (tar.gz archive)`,
            `curl -X POST \\`,
            `  -H "Authorization: Bearer ${authTokenOrPlaceholder(ctx)}" \\`,
            `  -H "Content-Type: application/gzip" \\`,
            `  --data-binary @module.tar.gz \\`,
            `  "${base}/proxy/${reg}/v1/modules/namespace/name/provider/1.0.0"`,
            ``,
            `# Download artifact URL returned as X-Terraform-Get header:`,
            `# ${base}/proxy/${reg}/v1/modules/namespace/name/provider/1.0.0/artifact`,
          ].join("\n");
        },
        note:
          `The response includes an ` +
          `<code class="font-mono bg-muted px-1 rounded">X-Terraform-Get</code> ` +
          `header pointing to the artifact download URL. Modules can also be yanked via the admin API.`,
      },
    ],
  },

  // ── RubyGems ───────────────────────────────────────────────────────────────
  {
    id: "rubygems",
    label: "RubyGems",
    fileHint: "RubyGems",
    description:
      `Mirror rubygems.org through this proxy for Bundler and the gem CLI. ` +
      `Gems are cached after the first download. Publish private gems with ` +
      `<code class="text-xs font-mono bg-muted px-1 rounded">gem push</code> when the registry ` +
      `is configured in <code class="text-xs font-mono bg-muted px-1 rounded">Local</code> ` +
      `or <code class="text-xs font-mono bg-muted px-1 rounded">Hybrid</code> mode.`,
    snippets: [
      {
        key: "gemsrc",
        label: "Bundler mirror / gem CLI source",
        lang: "bash",
        template: (ctx) => {
          const { base, registryName: reg } = ctx;
          const proxyUrl = withCredentials(`${base}/proxy/${reg}/`, ctx);
          return [
            `# Bundler — mirror rubygems.org through the proxy`,
            `# Run once, or commit to .bundle/config`,
            `bundle config set mirror.https://rubygems.org/ ${proxyUrl}`,
            ``,
            `# gem CLI — replace the default source`,
            `# gem sources --remove https://rubygems.org/`,
            `# gem sources --add ${proxyUrl}`,
          ].join("\n");
        },
        note:
          `The <code class="font-mono bg-muted px-1 rounded">bundle config</code> mirror setting ` +
          `intercepts all rubygems.org requests transparently — no changes to your ` +
          `<code class="font-mono bg-muted px-1 rounded">Gemfile</code> needed.`,
      },
      {
        key: "gem-publish",
        label: "Publish a private gem (Local / Hybrid mode)",
        lang: "bash",
        showWhen: isPublishMode,
        template: (ctx) => {
          const { base, registryName: reg, isAuthenticated, token } = ctx;
          const lines = [
            `# Publish a gem (local / hybrid mode only)`,
            `gem push name-version.gem --host ${base}/proxy/${reg}`,
          ];
          if (isAuthenticated) {
            lines.push(
              ``,
              `# Credentials: set GEM_HOST_API_KEY or pass --key`,
              `export GEM_HOST_API_KEY="${token}"`,
            );
          }
          return lines.join("\n");
        },
        note:
          `The registry must be configured with <code class="font-mono bg-muted px-1 rounded">mode = "local"</code> or ` +
          `<code class="font-mono bg-muted px-1 rounded">mode = "hybrid"</code> in ` +
          `<code class="font-mono bg-muted px-1 rounded">config.toml</code> to accept publishes.`,
      },
    ],
  },

  // ── Composer ───────────────────────────────────────────────────────────────
  {
    id: "composer",
    label: "Composer",
    fileHint: "Composer",
    description:
      `Proxy PHP Composer package downloads from ` +
      `<a href="https://packagist.org" target="_blank" rel="noopener" ` +
      `class="underline underline-offset-2 hover:text-foreground transition-colors">Packagist</a> ` +
      `or publish private packages via ZIP upload when the registry is configured in ` +
      `<code class="text-xs font-mono bg-muted px-1 rounded">Local</code> ` +
      `or <code class="text-xs font-mono bg-muted px-1 rounded">Hybrid</code> mode. ` +
      `Authentication uses <code class="text-xs font-mono bg-muted px-1 rounded">auth.json</code> ` +
      `(HTTP Basic) rather than a token header — this is a Composer convention.`,
    snippets: [
      {
        key: "composer-json",
        label: "composer.json — add the proxy as a repository",
        lang: "jsonc",
        template: (ctx) => {
          const { base, registryName: reg, isAuthenticated, token } = ctx;
          const lines = [
            `// composer.json — add inside the root object`,
            `"repositories": [`,
            `  {`,
            `    "type": "composer",`,
            `    "url": "${base}/proxy/${reg}/",`,
          ];
          if (isAuthenticated) {
            lines.push(
              `    "options": {`,
              `      "http": {`,
              `        "header": ["Authorization: Bearer ${token}"]`,
              `      }`,
              `    }`,
            );
          }
          lines.push(`  }`, `]`);
          return lines.join("\n");
        },
      },
      {
        key: "composer-auth",
        label: "auth.json — credentials (never commit this file)",
        lang: "jsonc",
        template: (ctx) => {
          let hostPart = ctx.base;
          try {
            hostPart = new URL(ctx.base).hostname;
          } catch {
            /* keep */
          }
          return [
            `// auth.json — project root or ~/.config/composer/auth.json`,
            `// Never commit this file!`,
            `{`,
            `  "http-basic": {`,
            `    "${hostPart}": {`,
            `      "username": "${ctx.isAuthenticated ? (ctx.netrcLogin ?? "user") : "user"}",`,
            `      "password": "${ctx.isAuthenticated ? ctx.token : "<your-token>"}"`,
            `    }`,
            `  }`,
            `}`,
          ].join("\n");
        },
        note:
          `Place <code class="font-mono bg-muted px-1 rounded">auth.json</code> in your project root or ` +
          `<code class="font-mono bg-muted px-1 rounded">~/.config/composer/auth.json</code> for global use. ` +
          `When present, Composer sends HTTP Basic credentials automatically — no ` +
          `<code class="font-mono bg-muted px-1 rounded">options.http.header</code> needed in ` +
          `<code class="font-mono bg-muted px-1 rounded">composer.json</code>.`,
      },
      {
        key: "composer-publish",
        label: "Publish a private package (Local / Hybrid mode)",
        lang: "bash",
        showWhen: isPublishMode,
        template: (ctx) => {
          const { base, registryName: reg } = ctx;
          const tok = authTokenOrPlaceholder(ctx);
          return [
            `# Publish a package (Local / Hybrid mode only)`,
            `# ZIP must contain composer.json with "name" (vendor/pkg) and "version"`,
            `zip -r vendor-pkg-1.0.0.zip vendor-pkg-1.0.0/`,
            ``,
            `curl -X POST \\`,
            `  -H "Authorization: Bearer ${tok}" \\`,
            `  -H "Content-Type: application/zip" \\`,
            `  --data-binary @vendor-pkg-1.0.0.zip \\`,
            `  "${base}/proxy/${reg}/api/upload"`,
            ``,
            `# Yank a version`,
            `curl -X DELETE \\`,
            `  -H "Authorization: Bearer ${tok}" \\`,
            `  "${base}/proxy/${reg}/api/packages/vendor/pkg/versions/1.0.0"`,
          ].join("\n");
        },
        note:
          `The ZIP must contain a valid <code class="font-mono bg-muted px-1 rounded">composer.json</code> ` +
          `at its root or inside a single top-level directory (GitHub archive layout). ` +
          `The <code class="font-mono bg-muted px-1 rounded">name</code> field must use the ` +
          `<code class="font-mono bg-muted px-1 rounded">vendor/package</code> format and the ` +
          `<code class="font-mono bg-muted px-1 rounded">version</code> field determines the published version.`,
      },
    ],
  },

  // ── PyPI ───────────────────────────────────────────────────────────────────
  {
    id: "pypi",
    label: "PyPI",
    fileHint: "PyPI",
    description:
      `Proxy <a href="https://pypi.org" target="_blank" rel="noopener" ` +
      `class="underline underline-offset-2 hover:text-foreground transition-colors">PyPI</a> ` +
      `through BatleHub for pip, uv, Poetry, and other Python package managers. ` +
      `Wheels and source distributions are cached after the first download. ` +
      `Publish private packages with <code class="text-xs font-mono bg-muted px-1 rounded">twine upload</code> ` +
      `when the registry is configured in <code class="text-xs font-mono bg-muted px-1 rounded">Local</code> ` +
      `or <code class="text-xs font-mono bg-muted px-1 rounded">Hybrid</code> mode.`,
    snippets: [
      {
        key: "pip-conf",
        label: "~/.pip/pip.conf — global pip configuration",
        lang: "ini",
        template: (ctx) => {
          const { base, registryName: reg, isAuthenticated, token, netrcLogin, netrcHost } = ctx;
          const lines = [
            `# ~/.pip/pip.conf  (Linux/macOS)`,
            String.raw`# %APPDATA%\pip\pip.ini  (Windows)`,
            `[global]`,
            `index-url = ${base}/proxy/${reg}/simple/`,
          ];
          if (isAuthenticated) {
            lines.push(
              ``,
              `# Credentials: use ~/.netrc (recommended) or embed in the URL:`,
              `# index-url = https://${netrcLogin}:${token}@${netrcHost}/proxy/${reg}/simple/`,
            );
          }
          return lines.join("\n");
        },
        note:
          `Alternatively, pass <code class="font-mono bg-muted px-1 rounded">--index-url</code> ` +
          `on the command line or set the ` +
          `<code class="font-mono bg-muted px-1 rounded">PIP_INDEX_URL</code> environment variable.`,
      },
      {
        key: "uv-index",
        label: "pyproject.toml — uv index configuration",
        lang: "toml",
        template: (ctx) => {
          const { base, registryName: reg, isAuthenticated, token, netrcLogin, netrcHost } = ctx;
          const lines = [
            `# pyproject.toml — add inside [tool.uv]`,
            `[[tool.uv.index]]`,
            `name = "batlehub"`,
            `url = "${base}/proxy/${reg}/simple/"`,
            `default = true`,
          ];
          if (isAuthenticated) {
            lines.push(
              ``,
              `# Credentials: uv reads ~/.netrc automatically`,
              `# machine ${netrcHost}`,
              `# login ${netrcLogin}`,
              `# password ${token}`,
            );
          }
          return lines.join("\n");
        },
      },
      {
        key: "twine-publish",
        label: "Publish a private package (Local / Hybrid mode)",
        lang: "bash",
        showWhen: isPublishMode,
        template: (ctx) => {
          const { base, registryName: reg } = ctx;
          const tok = authTokenOrPlaceholder(ctx);
          return [
            `# Publish a wheel or sdist (Local / Hybrid mode only)`,
            `# Build first: python -m build`,
            ``,
            `twine upload \\`,
            `  --repository-url ${base}/proxy/${reg}/legacy/ \\`,
            `  --username __token__ \\`,
            `  --password ${tok} \\`,
            `  dist/*`,
            ``,
            `# Or via ~/.pypirc:`,
            `# [batlehub]`,
            `# repository = ${base}/proxy/${reg}/legacy/`,
            `# username = __token__`,
            `# password = ${tok}`,
          ].join("\n");
        },
        note:
          `The registry must be configured with ` +
          `<code class="font-mono bg-muted px-1 rounded">mode = "local"</code> or ` +
          `<code class="font-mono bg-muted px-1 rounded">mode = "hybrid"</code>. ` +
          `The filename, name, and version are derived from the wheel or sdist metadata automatically.`,
      },
    ],
  },

  // ── Conda ──────────────────────────────────────────────────────────────────
  {
    id: "conda",
    label: "Conda",
    fileHint: "Conda",
    description:
      `Proxy conda channels (conda-forge, defaults, or custom) through BatleHub. ` +
      `<code class="text-xs font-mono bg-muted px-1 rounded">repodata.json</code> and package files ` +
      `are cached after the first request. Publish private conda packages in ` +
      `<code class="text-xs font-mono bg-muted px-1 rounded">Local</code> ` +
      `or <code class="text-xs font-mono bg-muted px-1 rounded">Hybrid</code> mode — packages ` +
      `appear in the channel's <code class="text-xs font-mono bg-muted px-1 rounded">repodata.json</code> automatically.`,
    snippets: [
      {
        key: "condarc",
        label: "~/.condarc — point conda at the proxy",
        lang: "yaml",
        template: (ctx) => {
          const { base, registryName: reg, isAuthenticated, token, netrcLogin, netrcHost } = ctx;
          const lines = [
            `# ~/.condarc  (or .condarc in the project root)`,
            `channels:`,
            `  - ${base}/proxy/${reg}`,
            `  - nodefaults`,
          ];
          if (isAuthenticated) {
            lines.push(
              ``,
              `# Credentials: conda reads ~/.netrc automatically`,
              `# machine ${netrcHost}`,
              `# login ${netrcLogin}`,
              `# password ${token}`,
            );
          }
          return lines.join("\n");
        },
        note:
          `Credentials are read automatically from ` +
          `<code class="font-mono bg-muted px-1 rounded">~/.netrc</code>. ` +
          `Set <code class="font-mono bg-muted px-1 rounded">ssl_verify: false</code> ` +
          `only for development with self-signed certificates.`,
      },
      {
        key: "conda-env",
        label: "environment.yml — reproducible environment",
        lang: "yaml",
        template: (ctx) =>
          [
            `# environment.yml`,
            `channels:`,
            `  - ${ctx.base}/proxy/${ctx.registryName}`,
            `  - nodefaults`,
            `dependencies:`,
            `  - python=3.11`,
            `  - numpy`,
          ].join("\n"),
      },
      {
        key: "conda-publish",
        label: "Publish a private conda package (Local / Hybrid mode)",
        lang: "bash",
        showWhen: isPublishMode,
        template: (ctx) => {
          const { base, registryName: reg } = ctx;
          const tok = authTokenOrPlaceholder(ctx);
          return [
            `# Publish a conda package (Local / Hybrid mode only)`,
            `# Build first: conda build my-recipe/`,
            ``,
            `curl -X POST \\`,
            `  -H "Authorization: Bearer ${tok}" \\`,
            `  -H "Content-Type: application/octet-stream" \\`,
            `  --data-binary @my-pkg-1.0.0-py311h0_0.tar.bz2 \\`,
            `  "${base}/proxy/${reg}/linux-64/"`,
            ``,
            `# Verify: repodata.json will list your package`,
            `curl -s "${base}/proxy/${reg}/linux-64/repodata.json" | \\`,
            `  python3 -c "import sys,json; d=json.load(sys.stdin); print(list(d['packages'].keys())[:5])"`,
          ].join("\n");
        },
        note:
          `Both <code class="font-mono bg-muted px-1 rounded">.tar.bz2</code> and ` +
          `<code class="font-mono bg-muted px-1 rounded">.conda</code> package formats are supported. ` +
          `The name, version, and build string are extracted from ` +
          `<code class="font-mono bg-muted px-1 rounded">info/index.json</code> inside the archive.`,
      },
    ],
  },

  // ── NuGet ──────────────────────────────────────────────────────────────────
  {
    id: "nuget",
    label: "NuGet",
    description:
      `Configure <code class="font-mono bg-muted px-1 rounded">dotnet</code> or ` +
      `<code class="font-mono bg-muted px-1 rounded">nuget.config</code> to use this proxy as a ` +
      `NuGet package source. Compatible with ` +
      `<code class="font-mono bg-muted px-1 rounded">dotnet add package</code>, ` +
      `<code class="font-mono bg-muted px-1 rounded">dotnet restore</code>, and ` +
      `<code class="font-mono bg-muted px-1 rounded">dotnet nuget push</code>.`,
    snippets: [
      {
        key: "nuget-source",
        label: "Add NuGet source (CLI)",
        lang: "bash",
        template: (ctx) => {
          const { base, registryName: reg, isAuthenticated } = ctx;
          const tok = authTokenOrPlaceholder(ctx);
          const lines = [
            `# Register the proxy as a NuGet source`,
            `dotnet nuget add source \\`,
            `  "${base}/proxy/${reg}/nuget/v3/index.json" \\`,
            `  --name ${reg}`,
          ];
          if (isAuthenticated) {
            lines.push(
              ``,
              `# Or with authentication`,
              `dotnet nuget add source \\`,
              `  "${base}/proxy/${reg}/nuget/v3/index.json" \\`,
              `  --name ${reg} \\`,
              `  --username __token__ --password ${tok}`,
            );
          }
          return lines.join("\n");
        },
      },
      {
        key: "nuget-config",
        label: "nuget.config (XML)",
        lang: "xml",
        template: (ctx) =>
          [
            `<?xml version="1.0" encoding="utf-8"?>`,
            `<configuration>`,
            `  <packageSources>`,
            `    <add key="${ctx.registryName}" value="${ctx.base}/proxy/${ctx.registryName}/nuget/v3/index.json" />`,
            `  </packageSources>`,
            `</configuration>`,
          ].join("\n"),
        note:
          `Place <code class="font-mono bg-muted px-1 rounded">nuget.config</code> in your project root ` +
          `or user profile (<code class="font-mono bg-muted px-1 rounded">~/.nuget/NuGet/NuGet.Config</code>).`,
      },
      {
        key: "nuget-publish",
        label: "Publish a package (Local / Hybrid mode only)",
        lang: "bash",
        showWhen: isPublishMode,
        template: (ctx) => {
          const { base, registryName: reg } = ctx;
          const tok = authTokenOrPlaceholder(ctx);
          return [
            `# Publish a .nupkg (Local / Hybrid mode only)`,
            `dotnet nuget push MyLib.1.0.0.nupkg \\`,
            `  --api-key ${tok} \\`,
            `  --source "${base}/proxy/${reg}/nuget/v3/index.json"`,
            ``,
            `# Yank a version`,
            `curl -X DELETE \\`,
            `  -H "Authorization: Bearer ${tok}" \\`,
            `  "${base}/proxy/${reg}/nuget/v2/package/mylib/1.0.0"`,
          ].join("\n");
        },
        note:
          `The registry accepts <code class="font-mono bg-muted px-1 rounded">.nupkg</code> files ` +
          `via <code class="font-mono bg-muted px-1 rounded">multipart/form-data</code> ` +
          `(as sent by <code class="font-mono bg-muted px-1 rounded">dotnet nuget push</code>). ` +
          `The <code class="font-mono bg-muted px-1 rounded">.nuspec</code> is automatically ` +
          `extracted from the archive to record package metadata.`,
      },
    ],
  },

  // ── Forgejo / Gitea (releases) ───────────────────────────────────────────────
  {
    id: "forgejo",
    label: "Forgejo",
    fileHint: "Releases",
    description:
      `Proxy release assets, source archives, and raw files from a ` +
      `<a href="https://forgejo.org" target="_blank" rel="noopener" ` +
      `class="underline underline-offset-2 hover:text-foreground transition-colors">Forgejo</a> ` +
      `or Gitea instance. Forgejo registries reuse the GitHub-style URL scheme.`,
    snippets: [
      {
        key: "forgejo-curl",
        label: "Download release assets & archives",
        lang: "bash",
        template: (ctx) => {
          const reg = `${ctx.base}/proxy/${ctx.registryName}`;
          const auth = ctx.isAuthenticated
            ? ` \\\n  -H "Authorization: Bearer ${ctx.token}"`
            : "";
          return [
            `# List releases for owner/repo`,
            `curl${auth} ${reg}/<owner>/<repo>/releases`,
            ``,
            `# Release metadata by tag`,
            `curl${auth} ${reg}/<owner>/<repo>/releases/tags/v1.0.0`,
            ``,
            `# Download a release asset by filename`,
            `curl -L -O${auth} ${reg}/<owner>/<repo>/releases/download/v1.0.0/app.tar.gz`,
            ``,
            `# Source tarball / zip for a tag, branch, or commit`,
            `curl -L -O${auth} ${reg}/<owner>/<repo>/tarball/v1.0.0`,
            `curl -L -O${auth} ${reg}/<owner>/<repo>/zipball/v1.0.0`,
            ``,
            `# Raw file`,
            `curl -L${auth} ${reg}/<owner>/<repo>/raw/main/README.md`,
            ``,
            `# Package registry passthrough (generic packages)`,
            `curl -L -O${auth} ${reg}/api/packages/<owner>/generic/<name>/<version>/<file>`,
          ].join("\n");
        },
        note:
          `Configure the upstream instance URL (e.g. ` +
          `<code class="font-mono bg-muted px-1 rounded">https://codeberg.org</code>) as the ` +
          `registry's upstream. The same adapter serves both Forgejo and Gitea. For ecosystem ` +
          `package registries (npm, Maven, PyPI, …) use the matching typed adapter pointed at the ` +
          `<code class="font-mono bg-muted px-1 rounded">/api/packages/{owner}/{type}</code> endpoint.`,
      },
    ],
  },

  // ── GitLab (releases) ────────────────────────────────────────────────────────
  {
    id: "gitlab",
    label: "GitLab",
    fileHint: "Releases",
    description:
      `Proxy GitLab releases, release link assets, and source archives. Project paths ` +
      `may include nested groups; the release sub-path is separated by ` +
      `<code class="text-xs font-mono bg-muted px-1 rounded">/-/</code>, mirroring GitLab's own URLs.`,
    snippets: [
      {
        key: "gitlab-curl",
        label: "Download releases & archives",
        lang: "bash",
        template: (ctx) => {
          const reg = `${ctx.base}/proxy/${ctx.registryName}`;
          const auth = ctx.isAuthenticated
            ? ` \\\n  -H "Authorization: Bearer ${ctx.token}"`
            : "";
          return [
            `# List releases for a project (nested groups allowed)`,
            `curl${auth} ${reg}/<group>/<project>/-/releases`,
            ``,
            `# Release metadata by tag`,
            `curl${auth} ${reg}/<group>/<project>/-/releases/v1.0.0`,
            ``,
            `# Download a release link asset (matched by link name)`,
            `curl -L -O${auth} ${reg}/<group>/<project>/-/releases/v1.0.0/downloads/app.bin`,
            ``,
            `# Source archive for a tag (format inferred from the extension)`,
            `curl -L -O${auth} ${reg}/<group>/<project>/-/archive/v1.0.0/source.tar.gz`,
            ``,
            `# Raw file from the repository`,
            `curl -L${auth} ${reg}/<group>/<project>/-/raw/main/README.md`,
            ``,
            `# Package registry passthrough (generic packages)`,
            `curl -L -O${auth} ${reg}/api/v4/projects/<id>/packages/generic/<name>/<version>/<file>`,
          ].join("\n");
        },
        note:
          `GitLab personal access tokens use the ` +
          `<code class="font-mono bg-muted px-1 rounded">PRIVATE-TOKEN</code> header — configure ` +
          `it as a custom upstream auth header on the registry. Set the upstream URL to your ` +
          `instance root (e.g. <code class="font-mono bg-muted px-1 rounded">https://gitlab.com</code>). ` +
          `For ecosystem package registries (npm, Maven, PyPI, …) use the matching typed adapter.`,
      },
    ],
  },

  // ── Debian APT (deb) ─────────────────────────────────────────────────────────
  {
    id: "deb",
    label: "Debian (APT)",
    fileHint: "/etc/apt/sources.list.d/",
    description:
      `Proxy and host Debian/Ubuntu APT repositories. In local/hybrid mode, publish ` +
      `<code class="text-xs font-mono bg-muted px-1 rounded">.deb</code> packages and BatleHub ` +
      `regenerates the <code class="text-xs font-mono bg-muted px-1 rounded">Packages</code>/` +
      `<code class="text-xs font-mono bg-muted px-1 rounded">Release</code> indexes (Ed25519 ` +
      `OpenPGP-signed when a key is configured).`,
    snippets: [
      {
        key: "apt-source",
        label: "APT source",
        lang: "bash",
        template: (ctx) => {
          const reg = `${ctx.base}/proxy/${ctx.registryName}/deb`;
          if (isPublishMode(ctx)) {
            // Local/hybrid: BatleHub signs Release with its own key (served at /key.gpg).
            return [
              `# Import BatleHub's repository signing key`,
              `curl -fsSL ${reg}/key.gpg | sudo tee /usr/share/keyrings/${ctx.registryName}.asc >/dev/null`,
              ``,
              `# Add the source (adjust suite/component to your repo)`,
              `echo "deb [signed-by=/usr/share/keyrings/${ctx.registryName}.asc] ${reg} stable main" \\`,
              `  | sudo tee /etc/apt/sources.list.d/${ctx.registryName}.list`,
              ``,
              `sudo apt update`,
            ].join("\n");
          }
          // Proxy: the upstream repo's own (relayed) signature is what apt verifies,
          // so the client must trust the UPSTREAM's archive key. For official
          // Debian/Ubuntu mirrors that key is already installed.
          return [
            `# Proxy mode relays the upstream repo's Release/InRelease and its signature.`,
            `# Verify with the UPSTREAM's archive key. Official Debian/Ubuntu mirrors`,
            `# ship it already (packages: debian-archive-keyring / ubuntu-keyring):`,
            `KEYRING=/usr/share/keyrings/debian-archive-keyring.gpg   # ubuntu: ubuntu-archive-keyring.gpg`,
            `echo "deb [signed-by=$KEYRING] ${reg} stable main" \\`,
            `  | sudo tee /etc/apt/sources.list.d/${ctx.registryName}.list`,
            ``,
            `# For a third-party upstream, import ITS key instead:`,
            `#   curl -fsSL <upstream-key-url> | gpg --dearmor \\`,
            `#     | sudo tee /usr/share/keyrings/${ctx.registryName}.gpg >/dev/null`,
            ``,
            `sudo apt update`,
          ].join("\n");
        },
        note: (ctx) =>
          isPublishMode(ctx)
            ? `For an unsigned local repository (no <code class="font-mono bg-muted px-1 rounded">repo_signing</code> key), replace ` +
              `<code class="font-mono bg-muted px-1 rounded">[signed-by=…]</code> with <code class="font-mono bg-muted px-1 rounded">[trusted=yes]</code>.`
            : `Proxy registries relay the upstream's signature, so apt verifies against the <strong>upstream's</strong> key — ` +
              `<code class="font-mono bg-muted px-1 rounded">${ctx.base}/proxy/${ctx.registryName}/deb/key.gpg</code> is not served (it is a local/hybrid signing artifact). ` +
              `A <code class="font-mono bg-muted px-1 rounded">NO_PUBKEY</code> error means that key isn't in the keyring named by ` +
              `<code class="font-mono bg-muted px-1 rounded">signed-by</code> — install <code class="font-mono bg-muted px-1 rounded">debian-archive-keyring</code>/` +
              `<code class="font-mono bg-muted px-1 rounded">ubuntu-keyring</code> (or import the upstream key), or use <code class="font-mono bg-muted px-1 rounded">[trusted=yes]</code>.`,
      },
      {
        key: "apt-publish",
        label: "Publish a .deb (local/hybrid)",
        lang: "bash",
        showWhen: isPublishMode,
        template: (ctx) =>
          [
            `# Upload to pool/{distribution}/{component}`,
            `curl -X PUT \\`,
            `  -H "Authorization: Bearer ${authTokenOrPlaceholder(ctx)}" \\`,
            `  --data-binary @hello_1.0_amd64.deb \\`,
            `  ${ctx.base}/proxy/${ctx.registryName}/deb/pool/stable/main/upload`,
          ].join("\n"),
      },
    ],
  },

  // ── RPM / YUM (rpm) ──────────────────────────────────────────────────────────
  {
    id: "rpm",
    label: "RPM (YUM/DNF)",
    fileHint: "/etc/yum.repos.d/",
    description:
      `Proxy and host RPM repositories for DNF/YUM. In local/hybrid mode, publish ` +
      `<code class="text-xs font-mono bg-muted px-1 rounded">.rpm</code> packages and BatleHub ` +
      `regenerates <code class="text-xs font-mono bg-muted px-1 rounded">repodata/</code> ` +
      `(Ed25519 OpenPGP-signed <code class="text-xs font-mono bg-muted px-1 rounded">repomd.xml.asc</code> ` +
      `when a key is configured).`,
    snippets: [
      {
        key: "dnf-repo",
        label: ".repo file",
        lang: "ini",
        template: (ctx) => {
          const reg = `${ctx.base}/proxy/${ctx.registryName}/rpm`;
          const lines = [`[${ctx.registryName}]`, `name=${ctx.registryName}`, `baseurl=${reg}`, `enabled=1`];
          if (isPublishMode(ctx)) {
            // Local/hybrid: repomd.xml.asc is signed by BatleHub's key (served at the URL below).
            lines.push(`repo_gpgcheck=1`, `gpgcheck=0`, `gpgkey=${reg}/repodata/repomd.xml.key`);
          } else {
            // Proxy: metadata (and any repomd.xml.asc) is relayed from upstream; there is
            // no BatleHub key. Verify against the upstream's key or disable the repo check.
            lines.push(`repo_gpgcheck=0`, `gpgcheck=0`, `# gpgkey=<upstream-project-gpg-key-url>`);
          }
          return lines.join("\n");
        },
        note: (ctx) =>
          isPublishMode(ctx)
            ? `Save to <code class="font-mono bg-muted px-1 rounded">/etc/yum.repos.d/${"{name}"}.repo</code>. ` +
              `For an unsigned local repo (no <code class="font-mono bg-muted px-1 rounded">repo_signing</code> key), set ` +
              `<code class="font-mono bg-muted px-1 rounded">repo_gpgcheck=0</code> and omit <code class="font-mono bg-muted px-1 rounded">gpgkey</code>.`
            : `Proxy registries have no BatleHub key — <code class="font-mono bg-muted px-1 rounded">repodata/repomd.xml.key</code> ` +
              `is only served for local/hybrid registries with a <code class="font-mono bg-muted px-1 rounded">repo_signing</code> key. ` +
              `To verify, point <code class="font-mono bg-muted px-1 rounded">gpgkey</code> at the upstream project's key and set ` +
              `<code class="font-mono bg-muted px-1 rounded">repo_gpgcheck=1</code>.`,
      },
      {
        key: "rpm-publish",
        label: "Publish a .rpm (local/hybrid)",
        lang: "bash",
        showWhen: isPublishMode,
        template: (ctx) =>
          [
            `curl -X PUT \\`,
            `  -H "Authorization: Bearer ${authTokenOrPlaceholder(ctx)}" \\`,
            `  --data-binary @hello-1.0-1.x86_64.rpm \\`,
            `  ${ctx.base}/proxy/${ctx.registryName}/rpm/upload`,
          ].join("\n"),
      },
    ],
  },
  // ── JetBrains IDE archives (proxy-only cache) ──────────────────────────────
  {
    id: "jetbrains",
    label: "JetBrains IDE",
    fileHint: "download.jetbrains.com",
    description:
      `Cache JetBrains IDE installer archives (proxy-only). The first download is ` +
      `streamed from <code class="text-xs font-mono bg-muted px-1 rounded">download.jetbrains.com</code> ` +
      `and cached; later downloads of the same file are served locally. ` +
      `IDE archives are large (~1–1.7 GB), so raise ` +
      `<code class="text-xs font-mono bg-muted px-1 rounded">[limits] max_artifact_size_bytes</code> ` +
      `(e.g. 2 GiB) or they will be rejected.`,
    snippets: [
      {
        key: "jetbrains-curl",
        label: "Download an IDE archive",
        lang: "bash",
        template: (ctx) => {
          const reg = `${ctx.base}/proxy/${ctx.registryName}/jetbrains`;
          const auth = ctx.isAuthenticated
            ? ` \\\n  -H "Authorization: Bearer ${ctx.token}"`
            : "";
          return [
            `# The path after /jetbrains/ maps to download.jetbrains.com/<path>`,
            `curl -fL -o ideaIC.tar.gz${auth} \\`,
            `  ${reg}/idea/ideaIC-2024.1.4.tar.gz`,
          ].join("\n");
        },
        note:
          `Use the same path as the upstream URL: ` +
          `<code class="font-mono bg-muted px-1 rounded">download.jetbrains.com/idea/ideaIC-2024.1.4.tar.gz</code> → ` +
          `<code class="font-mono bg-muted px-1 rounded">/proxy/{name}/jetbrains/idea/ideaIC-2024.1.4.tar.gz</code>. ` +
          `<code class="font-mono bg-muted px-1 rounded">download.jetbrains.com</code> redirects to a CDN ` +
          `(<code class="font-mono bg-muted px-1 rounded">download-cdn.jetbrains.com</code>) — the redirect is followed ` +
          `automatically, so always use the canonical path, not the CDN host. Use real archive names ` +
          `(<code class="font-mono bg-muted px-1 rounded">ideaIU-…</code> Ultimate / ` +
          `<code class="font-mono bg-muted px-1 rounded">ideaIC-…</code> Community).`,
      },
      {
        key: "jetbrains-config",
        label: "Server config",
        lang: "toml",
        template: (ctx) =>
          [
            `[limits]`,
            `max_artifact_size_bytes = 2147483648  # 2 GiB — IDE archives are large`,
            ``,
            `[[registries]]`,
            `name = "${ctx.registryName}"`,
            `type = "jetbrains"`,
            `mode = "proxy"            # upstream defaults to https://download.jetbrains.com`,
            ``,
            `[registries.rbac]`,
            `anonymous = ["releases:read"]`,
          ].join("\n"),
        note:
          `Override <code class="font-mono bg-muted px-1 rounded">upstreams</code> to cache another host ` +
          `(e.g. <code class="font-mono bg-muted px-1 rounded">https://plugins.jetbrains.com</code>).`,
      },
    ],
  },
];
