import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vitepress";

/**
 * The RFC status vocabulary, exactly as `internal/0000-rfc-template.md` defines
 * it. Longest first, because "In review" would otherwise never match against a
 * looser pattern, and "Superseded by NNNN" carries the number it points at.
 */
const RFC_STATUSES = [
  /^Superseded by (\d{4})\b/,
  /^In review\b/,
  /^Implemented\b/,
  /^Accepted\b/,
  /^Rejected\b/,
  /^Draft\b/,
];

/** A status the product can be described by: the page is history, not a proposal. */
const SETTLED = /^(Implemented|Rejected|Superseded)/;

/**
 * Read an RFC's status out of its own header table.
 *
 * Generated, never hand-written on the page. Every RFC already carries the fact
 * in `| Status | … |`; a second copy in frontmatter would be one more thing to
 * keep in step, and the whole of RFC 0005 is about facts maintained twice.
 *
 * Two things the parser has to survive, because the existing files already do
 * them. The value is prose rather than an enum — RFC 0001 reads
 * `**Implemented** — all phases landed; see the implementation notes in §13` —
 * so the leading token is the status and the remainder is a note worth
 * rendering with it. And `| Status |` occurs elsewhere: RFC 0003 has a row in a
 * body table whose first cell is "Status", which is why this stops at the first
 * `##` rather than scanning the file.
 */
function rfcStatus(srcDir: string, filePath: string) {
  const raw = readFileSync(join(srcDir, filePath), "utf8");
  const header = raw.split(/^## /m)[0];
  const row = header.match(/^\|\s*Status\s*\|([^|]*)\|/m);
  const value = row?.[1].replace(/\*\*/g, "").trim();

  if (!value) {
    throw new Error(
      `${filePath}: no parseable "| Status | … |" row in the header table. ` +
        `Every RFC needs one — an RFC published without a banner is exactly ` +
        `the page that misleads (RFC 0005 §6.8).`,
    );
  }

  const pattern = RFC_STATUSES.find((p) => p.test(value));
  if (!pattern) {
    throw new Error(
      `${filePath}: status "${value}" is not in the template's vocabulary ` +
        `(Draft, In review, Accepted, Implemented, Rejected, Superseded by NNNN).`,
    );
  }

  const [state] = value.match(pattern)!;
  const note = value.slice(state.length).replace(/^\s*[—–-]\s*/, "").trim();
  return { state, note, settled: SETTLED.test(state) };
}

export default defineConfig({
  appearance: "dark",
  title: "BatleHub",
  description:
    "Your package hub. Proxy, cache, and host npm, Cargo, Maven, PyPI, NuGet, Go, RubyGems, Terraform, and more.",
  cleanUrls: false,
  base: process.env.BASE_URL || "/",

  // Merging the two documentation trees is not the same as publishing both
  // (RFC 0005 §6.7). Three classes stay in the repo and out of the build:
  // generated artifacts (`i18n-review-fr.md` is output from
  // `task ui:i18n:review`, not a document), point-in-time security findings,
  // and forms — the RFC template is something you copy, not something you read.
  //
  // The rule is worth stating plainly, because the RFCs next door *do* publish
  // and they are candid about this project's own past defects: design history
  // publishes, security findings do not. Visible rigour about one's own
  // mistakes reads as competence; a dated vulnerability survey reads as a map.
  srcExclude: ["internal/**"],

  // Generated from each RFC's own `Status` row — see `rfcStatus` above and the
  // banner in `theme/RfcStatus.vue`. An unparseable status fails the build
  // rather than rendering an unlabelled page.
  transformPageData(pageData, ctx) {
    if (pageData.filePath.startsWith("rfc/") && pageData.filePath !== "rfc/index.md") {
      pageData.frontmatter.rfcStatus = rfcStatus(
        ctx.siteConfig.srcDir,
        pageData.filePath,
      );
    }
  },
  vite: {
    server: {
      allowedHosts: true,
      host: true,
    },
    plugins: [
      // Drop the default theme's Inter. See theme/no-inter.css for why.
      //
      // A `resolve.alias` entry cannot do this: aliases match the import
      // specifier as written, and the default theme imports its own stylesheet
      // as the relative `./styles/fonts.css`. Matching that string alone would
      // catch any file of that name in any package. Resolving by importer is
      // what makes the redirect specific to VitePress's own theme.
      {
        name: "batlehub:no-inter",
        enforce: "pre" as const,
        resolveId(source: string, importer?: string) {
          if (
            source.endsWith("styles/fonts.css") &&
            importer?.includes("theme-default")
          ) {
            return fileURLToPath(
              new URL("./theme/no-inter.css", import.meta.url),
            );
          }
          return null;
        },
      },
    ],
  },
  head: [
    [
      "link",
      {
        rel: "icon",
        type: "image/svg+xml",
        href: (process.env.BASE_URL || "/") + "logo.svg",
      },
    ],

    // The two specimen faces, self-hosted (RFC 0005 §6.4). Preloaded because
    // both are in the first paint: headings and the wordmark are Silkscreen and
    // every other string on the page is JetBrains Mono. Silkscreen 400 is not
    // preloaded — every Silkscreen rule in the theme asks for 700.
    //
    // `@font-face` declares these in `theme/vp-bridge.css` with root-absolute
    // paths; the preload hrefs carry BASE_URL for the same reason logo.svg does,
    // since the site publishes under a prefix.
    [
      "link",
      {
        rel: "preload",
        as: "font",
        type: "font/woff2",
        crossorigin: "",
        href: (process.env.BASE_URL || "/") + "fonts/silkscreen-700.woff2",
      },
    ],
    [
      "link",
      {
        rel: "preload",
        as: "font",
        type: "font/woff2",
        crossorigin: "",
        href: (process.env.BASE_URL || "/") + "fonts/jetbrainsmono-latin.woff2",
      },
    ],

    // The rendition sync. `tokens.css` authors light under
    // `:root[data-theme="light"]`; VitePress carries its resolved appearance as
    // a `.dark` class on the same element. This mirrors one onto the other so
    // there is a single stored preference and a single resolution — The
    // Stored-Preference Rule wants one mechanism, and VitePress already has it
    // (it stores `system|light|dark` and resolves before first paint).
    //
    // The initial call plus the observer covers both injection orders: if
    // VitePress's appearance script has already run, the first call is right; if
    // it has not, the observer fires when it does. Both happen in the head, and
    // observer callbacks are microtasks, so neither case reaches a paint with
    // the wrong ground. The filter is `class`, so writing `data-theme` cannot
    // re-trigger it.
    [
      "script",
      {},
      `(()=>{const e=document.documentElement,s=()=>e.setAttribute("data-theme",e.classList.contains("dark")?"dark":"light");s();new MutationObserver(s).observe(e,{attributes:true,attributeFilter:["class"]})})()`,
    ],
    ["meta", { name: "theme-color", content: "#dc2626" }],
    ["meta", { property: "og:type", content: "website" }],
    ["meta", { property: "og:site_name", content: "BatleHub" }],
    ["meta", { property: "og:title", content: "BatleHub" }],
    [
      "meta",
      {
        property: "og:description",
        content:
          "Your package hub. Proxy, cache, and host npm, Cargo, Maven, PyPI, NuGet, Go, RubyGems, Terraform, and more.",
      },
    ],
    [
      "meta",
      {
        property: "og:image",
        content: (process.env.BASE_URL || "/") + "logo.svg",
      },
    ],
    ["meta", { name: "twitter:card", content: "summary" }],
    ["meta", { name: "twitter:title", content: "BatleHub" }],
    [
      "meta",
      {
        name: "twitter:description",
        content:
          "Your package hub. Proxy, cache, and host npm, Cargo, Maven, PyPI, NuGet, Go, RubyGems, Terraform, and more.",
      },
    ],
    [
      "meta",
      {
        name: "twitter:image",
        content: (process.env.BASE_URL || "/") + "logo.svg",
      },
    ],
  ],

  markdown: {
    // Syntax highlighting is the one colour system on this site the design
    // tokens do not decide — a highlighter's palette is a mapping from grammar
    // to hue, and DESIGN.md's four colours cannot express one. What it *can*
    // decide is the floor: every token has to clear AA against the ground the
    // code block actually sits on, which is `--ground-sunk` in both renditions.
    //
    // VitePress's defaults do not. Measured by `docs:design:rendered`:
    // `github-dark`'s comment token (#6a737d) lands at 4.35:1 on near-black and
    // `github-light`'s string token (#22863a) at 4.49:1 on paper — the second
    // under the bar by a hundredth, which is exactly the kind of miss nothing
    // but a measurement finds. GitHub's Primer defaults fix the dark ground and
    // still leave the light comment at 4.41:1, because this world's paper is
    // `oklch(0.99 0.004 18)` rather than #ffffff and a palette tuned against
    // pure white does not survive the move. `light-plus`/`dark-plus` clear both.
    theme: { light: "github-light-high-contrast", dark: "github-dark-high-contrast" },
  },

  themeConfig: {
    logo: "/logo.svg",
    siteTitle: "BatleHub.",

    // Navbar = top-level section entry points (plain links). The detailed
    // page tree for each section lives in the sidebar, so the two don't repeat
    // the same items. Reference topics (Caching, Access Control, Package
    // Explorer, SBOM, HA) and Config Generator are reached via the guide sidebar.
    // No "Home" entry: the wordmark to its left already links there, and the
    // navbar has exactly as much room as it has. Spending a slot on the one
    // destination every visitor can already reach is what pushed the appearance
    // toggle off the edge when the Config Generator was added.
    nav: [
      {
        text: "Install",
        link: "/guide/installation",
        activeMatch: "/guide/installation",
      },
      // The one page that is a tool rather than a document. It is in the navbar
      // because a visitor who has decided to install needs a `config.toml`
      // next, and generating one beats reading a 12 000-word reference to
      // write it by hand — which is where it was buried until now.
      {
        text: "Config Generator",
        link: "/guide/config-generator",
        activeMatch: "/guide/config-generator",
      },
      {
        text: "User Guide",
        link: "/use/",
        activeMatch: "/use/",
      },
      {
        text: "Registries",
        link: "/registries/",
        activeMatch: "/registries/",
      },
      {
        text: "Admin",
        link: "/guide/administration",
        activeMatch: "/guide/admin",
      },
      {
        text: "Operations",
        link: "/operations/",
        activeMatch: "/operations/",
      },
      // Three sections that are about the project rather than about running it.
      // A dropdown rather than three more top-level entries: the navbar is the
      // list of things a visitor came for, and nobody arrives at a package
      // proxy's documentation wanting the RFC index.
      {
        text: "Project",
        items: [
          { text: "Roadmap", link: "/guide/roadmap" },
          { text: "Contributing", link: "/contributing/" },
          { text: "Design history", link: "/rfc/" },
        ],
        activeMatch: "/(contributing|rfc)/",
      },
    ],

    // Page-grouped sidebars: each entry links a sibling *page*, not an in-page
    // anchor. VitePress's on-this-page outline (right aside) covers the headings
    // within a page, so they are not duplicated in the left sidebar.
    sidebar: {
      "/registries/": [
        {
          text: "Registries",
          items: [{ text: "Overview & matrix", link: "/registries/" }],
        },
        {
          text: "Source hosting",
          items: [
            { text: "GitHub", link: "/registries/github" },
            { text: "Forgejo / Gitea", link: "/registries/forgejo" },
            { text: "GitLab", link: "/registries/gitlab" },
          ],
        },
        {
          text: "Language package managers",
          items: [
            { text: "npm", link: "/registries/npm" },
            { text: "Cargo", link: "/registries/cargo" },
            { text: "Go Modules", link: "/registries/goproxy" },
            { text: "Maven", link: "/registries/maven" },
            { text: "PyPI", link: "/registries/pypi" },
            { text: "Conda", link: "/registries/conda" },
            { text: "Composer", link: "/registries/composer" },
            { text: "RubyGems", link: "/registries/rubygems" },
            { text: "NuGet", link: "/registries/nuget" },
            { text: "Terraform", link: "/registries/terraform" },
          ],
        },
        {
          text: "Editor extensions",
          items: [
            { text: "OpenVSX", link: "/registries/openvsx" },
            { text: "VS Code Marketplace", link: "/registries/vscode-marketplace" },
            { text: "JetBrains Marketplace", link: "/registries/jetbrains-marketplace" },
          ],
        },
        {
          text: "OS / system packages",
          items: [
            { text: "Debian / APT", link: "/registries/deb" },
            { text: "RPM / YUM / DNF", link: "/registries/rpm" },
            { text: "Pacman / Arch", link: "/registries/pacman" },
          ],
        },
        {
          text: "Binaries & mirrors",
          items: [
            { text: "JetBrains IDEs", link: "/registries/jetbrains" },
            { text: "Generic mirror", link: "/registries/generic" },
          ],
        },
      ],
      // I run this server.
      "/guide/": [
        {
          text: "Getting started",
          items: [
            { text: "What it does", link: "/guide/features" },
            { text: "Installation", link: "/guide/installation" },
            { text: "Configuration", link: "/guide/configuration" },
            { text: "Worked examples", link: "/guide/configuration-examples" },
            { text: "Config Generator", link: "/guide/config-generator" },
          ],
        },
        {
          text: "Administration",
          items: [
            { text: "Overview", link: "/guide/administration" },
            { text: "Configuration", link: "/guide/admin-config" },
            { text: "Storage & health", link: "/guide/admin-storage-health" },
            { text: "Policies & packages", link: "/guide/admin-policies" },
            { text: "Access & audit", link: "/guide/admin-access" },
          ],
        },
        {
          text: "Reference",
          items: [
            { text: "Caching", link: "/guide/caching" },
            { text: "Access Control", link: "/guide/access-control" },
            { text: "Host-based routing", link: "/guide/host-routing" },
            { text: "Private upstreams", link: "/guide/private-upstreams" },
            { text: "Hot reload", link: "/guide/hot-reload" },
            { text: "SBOM", link: "/guide/sbom" },
            { text: "High Availability", link: "/guide/high-availability" },
            { text: "Server binary subcommands", link: "/guide/server-cli" },
            { text: "Capacity planning", link: "/guide/capacity-planning" },
          ],
        },
        {
          text: "Project",
          items: [{ text: "Roadmap", link: "/guide/roadmap" }],
        },
      ],

      // I have a package manager and a token, and I need this to work.
      "/use/": [
        {
          text: "Using BatleHub",
          items: [
            { text: "Overview", link: "/use/" },
            { text: "Publishing packages", link: "/use/publishing" },
            { text: "Command-line client", link: "/use/cli" },
          ],
        },
        {
          text: "Package Explorer",
          items: [
            { text: "Overview", link: "/use/package-explorer" },
            { text: "Upstream search", link: "/use/package-explorer-search" },
            { text: "Access control", link: "/use/package-explorer-access" },
            { text: "Cache & API", link: "/use/package-explorer-cache" },
          ],
        },
        {
          text: "When something is wrong",
          items: [
            { text: "Vulnerability proxy", link: "/use/vulnerability-proxy" },
            { text: "Troubleshooting", link: "/use/troubleshooting" },
          ],
        },
      ],

      // Something is broken, or an auditor is asking.
      "/operations/": [
        {
          text: "Operations",
          items: [{ text: "Overview", link: "/operations/" }],
        },
        {
          text: "Runbooks",
          items: [
            { text: "Incident response", link: "/operations/incident-response" },
            { text: "Disaster recovery", link: "/operations/disaster-recovery" },
            { text: "What leaves this instance", link: "/operations/egress" },
            {
              text: "Production hardening",
              link: "/operations/production-hardening",
            },
            { text: "Registry health check", link: "/operations/check-registries" },
          ],
        },
        {
          text: "Compliance",
          items: [
            { text: "Change management", link: "/operations/change-management" },
            { text: "SOC 2 checklist", link: "/operations/soc2-checklist" },
          ],
        },
      ],

      // Someone changing the code.
      "/contributing/": [
        {
          text: "Contributing",
          items: [
            { text: "Overview", link: "/contributing/" },
            { text: "Working on BatleHub", link: "/contributing/contributing" },
            { text: "Testing", link: "/contributing/testing" },
            { text: "Vulnerability scanning & SBOMs", link: "/contributing/security-scanning" },
          ],
        },
        {
          text: "Extending",
          items: [
            { text: "Adding a registry", link: "/contributing/adding-a-registry" },
            {
              text: "Adding a vulnerability scanner",
              link: "/contributing/adding-a-vulnerability-scanner",
            },
          ],
        },
      ],

      // Someone asking why it is like this. Ordered oldest-first, because these
      // read as a sequence: each one argues with the state the previous left.
      "/rfc/": [
        {
          text: "Design history",
          items: [
            { text: "What these are", link: "/rfc/" },
            { text: "0001 — Subdomain routing", link: "/rfc/0001-subdomain-routing" },
            {
              text: "0002 — Vulnerability flags",
              link: "/rfc/0002-vulnerability-flags-and-exposure",
            },
            { text: "0003 — UI rework", link: "/rfc/0003-ui-rework" },
            {
              text: "0004 — Admin composition",
              link: "/rfc/0004-admin-composition-and-api-surface",
            },
            {
              text: "0004-bis — What 0004 left",
              link: "/rfc/0004-bis-what-rfc-0004-left",
            },
            {
              text: "0005 — One documentation tree",
              link: "/rfc/0005-docs-site-design-system",
            },
            {
              text: "0005-bis — Two readers, one home each",
              link: "/rfc/0005-bis-audience-split-and-one-home",
            },
            {
              text: "0006 — A block every ecosystem can see",
              link: "/rfc/0006-blocked-versions-hidden-everywhere",
            },
            {
              text: "0007 — The README, per version",
              link: "/rfc/0007-package-readmes",
            },
            {
              text: "0007-bis — The three 0007 deferred",
              link: "/rfc/0007-bis-images-search-and-fetch",
            },
            {
              text: "0008 — mise in an air-gapped estate",
              link: "/rfc/0008-mise-in-an-air-gapped-estate",
            },
            {
              text: "0009 — Every endpoint the client actually calls",
              link: "/rfc/0009-protocol-coverage",
            },
          ],
        },
      ],
    },

    socialLinks: [
      { icon: "git", link: "https://git.batleforc.fr/batleforc/batlehub" },
      { icon: "github", link: "https://github.com/batleforc/batlehub" },
    ],

    footer: {
      message:
        "Released under the Apache 2.0 License. Made with ❤️ and too much ☕.",
      copyright: "Copyright © 2026 Batleforc",
    },

    search: {
      provider: "local",
    },
  },
});
