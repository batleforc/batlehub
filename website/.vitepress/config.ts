import { defineConfig } from "vitepress";

export default defineConfig({
  appearance: "dark",
  title: "BatleHub",
  description:
    "Your package hub. Proxy, cache, and host npm, Cargo, Maven, PyPI, NuGet, Go, RubyGems, Terraform, and more.",
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

  themeConfig: {
    logo: "/logo.svg",
    siteTitle: "BatleHub.",

    // Navbar = top-level section entry points (plain links). The detailed
    // page tree for each section lives in the sidebar, so the two don't repeat
    // the same items. Reference topics (Caching, Access Control, Package
    // Explorer, SBOM, HA) and Config Generator are reached via the guide sidebar.
    nav: [
      { text: "Home", link: "/" },
      {
        text: "Installation",
        link: "/guide/installation",
        activeMatch: "/guide/installation",
      },
      {
        text: "User Guide",
        link: "/guide/user",
        activeMatch: "/guide/user",
      },
      {
        text: "Registries",
        link: "/registries/",
        activeMatch: "/registries/",
      },
      {
        text: "Administration",
        link: "/guide/administration",
        activeMatch: "/guide/admin",
      },
      {
        text: "Roadmap",
        link: "/guide/roadmap",
        activeMatch: "/guide/roadmap",
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
          collapsed: false,
          items: [
            { text: "GitHub", link: "/registries/github" },
            { text: "Forgejo / Gitea", link: "/registries/forgejo" },
            { text: "GitLab", link: "/registries/gitlab" },
          ],
        },
        {
          text: "Language package managers",
          collapsed: false,
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
          collapsed: false,
          items: [
            { text: "OpenVSX", link: "/registries/openvsx" },
            { text: "VS Code Marketplace", link: "/registries/vscode-marketplace" },
            { text: "JetBrains Marketplace", link: "/registries/jetbrains-marketplace" },
          ],
        },
        {
          text: "OS / system packages",
          collapsed: false,
          items: [
            { text: "Debian / APT", link: "/registries/deb" },
            { text: "RPM / YUM / DNF", link: "/registries/rpm" },
            { text: "Pacman / Arch", link: "/registries/pacman" },
          ],
        },
        {
          text: "Binaries & mirrors",
          collapsed: false,
          items: [
            { text: "JetBrains IDEs", link: "/registries/jetbrains" },
            { text: "Generic mirror", link: "/registries/generic" },
          ],
        },
      ],
      "/guide/": [
        {
          text: "Getting started",
          items: [
            { text: "Installation", link: "/guide/installation" },
            { text: "User Guide", link: "/guide/user" },
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
            {
              text: "Package Explorer",
              link: "/guide/package-explorer",
              items: [
                { text: "Upstream search", link: "/guide/package-explorer-search" },
                { text: "Access control", link: "/guide/package-explorer-access" },
                { text: "Cache & API", link: "/guide/package-explorer-cache" },
              ],
            },
            { text: "SBOM", link: "/guide/sbom" },
            { text: "High Availability", link: "/guide/high-availability" },
          ],
        },
        {
          text: "Project",
          items: [{ text: "Roadmap", link: "/guide/roadmap" }],
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
