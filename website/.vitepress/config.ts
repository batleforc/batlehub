import { defineConfig } from "vitepress";

export default defineConfig({
  title: "BatleHub",
  description:
    "Your package hub. Proxy, cache, and host npm, Cargo, Go, Maven, Terraform, and RubyGems registries.",
  cleanUrls: true,
  base: process.env.BASE_URL || "/",

  head: [
    ["link", { rel: "icon", type: "image/svg+xml", href: "/logo.svg" }],
    ["meta", { name: "theme-color", content: "#646cff" }],
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
    ["meta", { property: "og:image", content: "/logo.svg" }],
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
    ["meta", { name: "twitter:image", content: "/logo.svg" }],
  ],

  themeConfig: {
    logo: "/logo.svg",

    nav: [
      { text: "Home", link: "/" },
      {
        text: "Installation",
        link: "/guide/installation",
        activeMatch: "/guide/installation",
      },
      {
        text: "Administration",
        link: "/guide/administration",
        activeMatch: "/guide/administration",
      },
      {
        text: "Caching",
        link: "/guide/caching",
        activeMatch: "/guide/caching",
      },
      { text: "User Guide", link: "/guide/user", activeMatch: "/guide/user" },
      {
        text: "Config Generator",
        link: "/guide/config-generator",
        activeMatch: "/guide/config-generator",
      },
      {
        text: "Access Control",
        link: "/guide/access-control",
        activeMatch: "/guide/access-control",
      },
      {
        text: "Roadmap",
        link: "/guide/roadmap",
        activeMatch: "/guide/roadmap",
      },
      {
        text: "GitHub",
        link: "https://git.batleforc.fr/batleforc/batlehub",
        target: "_blank",
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
    ],

    footer: {
      message: "Released under the MIT License.",
      copyright: "Copyright © 2025 Batleforc",
    },

    search: {
      provider: "local",
    },
  },
});
