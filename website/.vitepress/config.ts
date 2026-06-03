import { defineConfig } from "vitepress";

export default defineConfig({
  appearance: "dark",
  title: "BatleHub",
  description:
    "Your package hub. Proxy, cache, and host npm, Cargo, Go, Maven, Terraform, and RubyGems registries.",
  cleanUrls: false,
  base: process.env.BASE_URL || "/",
  vite: {
    server: {
      allowedHosts: true,
    },
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
    ["meta", { name: "theme-color", content: "#dc2626" }],
    ["meta", { property: "og:type", content: "website" }],
    ["meta", { property: "og:site_name", content: "BatleHub" }],
    ["meta", { property: "og:title", content: "BatleHub" }],
    [
      "meta",
      {
        property: "og:description",
        content:
          "Your package hub. Proxy, cache, and host npm, Cargo, Go, Maven, Terraform, and RubyGems registries.",
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
          "Your package hub. Proxy, cache, and host npm, Cargo, Go, Maven, Terraform, and RubyGems registries.",
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

  themeConfig: {
    logo: "/logo.svg",
    siteTitle: "BatleHub.",

    nav: [
      { text: "Home", link: "/" },
      {
        text: "Getting Started",
        activeMatch: "/guide/(installation|user)",
        items: [
          { text: "Installation", link: "/guide/installation" },
          { text: "User Guide", link: "/guide/user" },
        ],
      },
      {
        text: "Reference",
        activeMatch: "/guide/(administration|caching|access-control|high-availability|package-explorer|sbom|explore-cache)",
        items: [
          { text: "Administration", link: "/guide/administration" },
          { text: "Caching", link: "/guide/caching" },
          { text: "Access Control", link: "/guide/access-control" },
          { text: "Package Explorer", link: "/guide/package-explorer" },
          { text: "SBOM", link: "/guide/sbom" },
          { text: "High Availability", link: "/guide/high-availability" },
        ],
      },
      {
        text: "Config Generator",
        activeMatch: "/guide/config-generator",
        link: "/guide/config-generator",
      },
      {
        text: "Roadmap",
        link: "/guide/roadmap",
        activeMatch: "/guide/roadmap",
      },
    ],

    // Per-page sidebar: each section only shows its own subsections.
    // Top nav handles cross-section navigation.
    sidebar: {
      "/guide/installation": [
        {
          text: "Installation",
          items: [
            {
              text: "Prerequisites",
              link: "/guide/installation#prerequisites",
            },
            {
              text: "Docker Compose",
              link: "/guide/installation#docker-compose-quickest-path",
            },
            {
              text: "Binary from source",
              link: "/guide/installation#binary-from-source",
            },
            { text: "Helm chart", link: "/guide/installation#helm-chart" },
            {
              text: "Helm: secret injection",
              link: "/guide/installation#helm-env-vars",
            },
            {
              text: "First-time setup",
              link: "/guide/installation#first-time-setup",
            },
          ],
        },
      ],
      "/guide/administration": [
        {
          text: "Administration",
          items: [
            {
              text: "Configuration",
              link: "/guide/administration#configuration",
            },
            {
              text: "Secret injection (${VAR})",
              link: "/guide/administration#env-inline",
            },
            {
              text: "Named env overrides",
              link: "/guide/administration#env-named",
            },
            { text: "Storage", link: "/guide/administration#storage" },
            {
              text: "Health & Observability",
              link: "/guide/administration#health",
            },
            {
              text: "Cache policy",
              link: "/guide/administration#cache-policy",
            },
            {
              text: "Package management",
              link: "/guide/administration#package-management",
            },
            { text: "Audit log", link: "/guide/administration#audit-log" },
            {
              text: "Beta channel",
              link: "/guide/administration#beta-channel",
            },
            { text: "IP blocking", link: "/guide/administration#ip-blocking" },
            {
              text: "Team namespaces",
              link: "/guide/administration#team-namespaces",
            },
            { text: "Rules", link: "/guide/administration#rules" },
          ],
        },
      ],
      "/guide/caching": [
        {
          text: "Caching",
          items: [
            {
              text: "How the cache works",
              link: "/guide/caching#how-the-cache-works",
            },
            {
              text: "Cache backend [cache]",
              link: "/guide/caching#cache-backend",
            },
            {
              text: "Per-registry policy",
              link: "/guide/caching#registry-cache-policy",
            },
            { text: "Cache warming", link: "/guide/caching#cache-warming" },
            { text: "Deduplication", link: "/guide/caching#deduplication" },
            { text: "Rate limiting", link: "/guide/caching#rate-limiting" },
            { text: "Worked examples", link: "/guide/caching#worked-examples" },
          ],
        },
      ],
      "/guide/user": [
        {
          text: "User Guide",
          items: [
            { text: "Getting a token", link: "/guide/user#getting-a-token" },
            { text: "npm", link: "/guide/user#npm" },
            { text: "Cargo", link: "/guide/user#cargo" },
            { text: "Go Modules", link: "/guide/user#go-modules" },
            {
              text: "VS Code Extensions",
              link: "/guide/user#vs-code-extensions",
            },
            { text: "Composer (PHP)", link: "/guide/user#composer" },
            { text: "PyPI (Python)", link: "/guide/user#pypi" },
            { text: "Conda", link: "/guide/user#conda" },
            { text: "Troubleshooting", link: "/guide/user#troubleshooting" },
          ],
        },
      ],
      "/guide/access-control": [
        {
          text: "Access Control",
          items: [
            {
              text: "Beta/Pre-Release Channel",
              link: "/guide/access-control#beta-channel",
            },
            {
              text: "How it works",
              link: "/guide/access-control#beta-how-it-works",
            },
            {
              text: "Configuration",
              link: "/guide/access-control#beta-config",
            },
            {
              text: "Managing members",
              link: "/guide/access-control#beta-members",
            },
            {
              text: "Registry support",
              link: "/guide/access-control#beta-registries",
            },
            {
              text: "IP-Based Blocking",
              link: "/guide/access-control#ip-blocking",
            },
            {
              text: "Configuration",
              link: "/guide/access-control#ip-config",
            },
            {
              text: "Manual block management",
              link: "/guide/access-control#ip-admin",
            },
            {
              text: "Storage backends",
              link: "/guide/access-control#ip-storage",
            },
            {
              text: "Team Namespaces & Visibility",
              link: "/guide/access-control#team-namespaces",
            },
            {
              text: "Namespace claims",
              link: "/guide/access-control#ns-claims",
            },
            {
              text: "Package visibility",
              link: "/guide/access-control#ns-visibility",
            },
            {
              text: "Registry support",
              link: "/guide/access-control#ns-registries",
            },
          ],
        },
      ],
      "/guide/package-explorer": [
        {
          text: "Package Explorer",
          items: [
            { text: "Overview", link: "/guide/package-explorer#overview" },
            { text: "Data sources", link: "/guide/package-explorer#sources" },
            { text: "Using the catalog", link: "/guide/package-explorer#catalog" },
            { text: "Package detail", link: "/guide/package-explorer#detail" },
            { text: "Firewall status", link: "/guide/package-explorer#firewall" },
            { text: "Upstream search", link: "/guide/package-explorer#upstream-search" },
            { text: "Search URL config", link: "/guide/package-explorer#search-url-config" },
            { text: "Access control", link: "/guide/package-explorer#access-control" },
            { text: "RBAC configuration", link: "/guide/package-explorer#rbac-config" },
            { text: "REST API", link: "/guide/package-explorer#api" },
            {
              text: "Explorer cache",
              link: "/guide/package-explorer#cache",
              items: [
                { text: "How it works", link: "/guide/package-explorer#cache-how" },
                { text: "Stale-while-unavailable", link: "/guide/package-explorer#cache-stale" },
                { text: "Auto-invalidation", link: "/guide/package-explorer#cache-auto-invalidate" },
                { text: "Manual flush (UI + API)", link: "/guide/package-explorer#cache-admin" },
                { text: "Multi-instance", link: "/guide/package-explorer#cache-ha" },
              ],
            },
            { text: "Performance notes", link: "/guide/package-explorer#performance" },
          ],
        },
      ],
      "/guide/sbom": [
        {
          text: "SBOM",
          items: [
            { text: "Overview", link: "/guide/sbom#overview" },
            { text: "Supported formats", link: "/guide/sbom#formats" },
            { text: "How SBOMs are generated", link: "/guide/sbom#generation" },
            { text: "Configuration", link: "/guide/sbom#configuration" },
            { text: "Per-artifact API", link: "/guide/sbom#per-artifact-api" },
            { text: "Org-level export", link: "/guide/sbom#org-export" },
            { text: "Admin UI", link: "/guide/sbom#admin-ui" },
            { text: "PURL mapping", link: "/guide/sbom#purl" },
            { text: "Worked examples", link: "/guide/sbom#examples" },
          ],
        },
      ],
      "/guide/high-availability": [
        {
          text: "High Availability",
          items: [
            {
              text: "Architecture overview",
              link: "/guide/high-availability#overview",
            },
            {
              text: "Prerequisites",
              link: "/guide/high-availability#prerequisites",
            },
            {
              text: "Configuration changes",
              link: "/guide/high-availability#config",
            },
            {
              text: "Docker Compose",
              link: "/guide/high-availability#compose",
            },
            {
              text: "Kubernetes / Helm",
              link: "/guide/high-availability#kubernetes",
            },
            {
              text: "Rolling updates",
              link: "/guide/high-availability#rolling-updates",
            },
            {
              text: "Health probes",
              link: "/guide/high-availability#health",
            },
            {
              text: "Observability",
              link: "/guide/high-availability#observability",
            },
            {
              text: "Known limitations",
              link: "/guide/high-availability#limitations",
            },
          ],
        },
      ],
      "/guide/roadmap": [
        {
          text: "Roadmap",
          items: [
            {
              text: "New registry types",
              link: "/guide/roadmap#new-registries",
            },
            { text: "Cache policy", link: "/guide/roadmap#cache-policy" },
            { text: "Metrics & observability", link: "/guide/roadmap#metrics" },
            { text: "Artifact integrity", link: "/guide/roadmap#integrity" },
            { text: "Rate limiting", link: "/guide/roadmap#rate-limiting" },
            { text: "Quota management", link: "/guide/roadmap#quotas" },
            { text: "Hot reload & config", link: "/guide/roadmap#hot-reload" },
            { text: "Webhooks", link: "/guide/roadmap#webhooks" },
            {
              text: "Private registry",
              link: "/guide/roadmap#private-registry",
            },
            { text: "SBOM", link: "/guide/roadmap#sbom" },
            { text: "UI improvements", link: "/guide/roadmap#ui" },
            { text: "Testing", link: "/guide/roadmap#testing" },
          ],
        },
      ],
    },

    socialLinks: [
      { icon: "git", link: "https://git.batleforc.fr/batleforc/batlehub" },
      { icon: "github", link: "https://github.com/batleforc/batlehub" },
    ],

    footer: {
      message: "Released under the MIT License.",
      copyright: "Copyright © 2026 Batleforc",
    },

    search: {
      provider: "local",
    },
  },
});
