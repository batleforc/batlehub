import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'BatleHub',
  description: 'Smart proxy and cache for package registries',
  cleanUrls: true,

  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: '/logo.svg' }],
  ],

  themeConfig: {
    logo: '/logo.svg',

    nav: [
      { text: 'Home',             link: '/' },
      { text: 'Installation',     link: '/guide/installation',      activeMatch: '/guide/installation' },
      { text: 'Administration',   link: '/guide/administration',    activeMatch: '/guide/administration' },
      { text: 'User Guide',       link: '/guide/user',              activeMatch: '/guide/user' },
      { text: 'Config Generator', link: '/guide/config-generator',  activeMatch: '/guide/config-generator' },
      { text: 'GitHub',           link: 'https://git.batleforc.fr/batleforc/batlehub', target: '_blank' },
    ],

    // Per-page sidebar: each section only shows its own subsections.
    // Top nav handles cross-section navigation.
    sidebar: {
      '/guide/installation': [
        {
          text: 'Installation',
          items: [
            { text: 'Prerequisites',       link: '/guide/installation#prerequisites' },
            { text: 'Docker Compose',      link: '/guide/installation#docker-compose-quickest-path' },
            { text: 'Binary from source',  link: '/guide/installation#binary-from-source' },
            { text: 'Helm chart',          link: '/guide/installation#helm-chart' },
            { text: 'First-time setup',    link: '/guide/installation#first-time-setup' },
          ],
        },
      ],
      '/guide/administration': [
        {
          text: 'Administration',
          items: [
            { text: 'Configuration',        link: '/guide/administration#configuration' },
            { text: 'Storage',              link: '/guide/administration#storage' },
            { text: 'Health & Observability', link: '/guide/administration#health' },
            { text: 'Package management',   link: '/guide/administration#package-management' },
            { text: 'Audit log',            link: '/guide/administration#audit-log' },
            { text: 'Rules',                link: '/guide/administration#rules' },
          ],
        },
      ],
      '/guide/user': [
        {
          text: 'User Guide',
          items: [
            { text: 'Getting a token',        link: '/guide/user#getting-a-token' },
            { text: 'npm',                    link: '/guide/user#npm' },
            { text: 'Cargo',                  link: '/guide/user#cargo' },
            { text: 'Go Modules',             link: '/guide/user#go-modules' },
            { text: 'VS Code Extensions',     link: '/guide/user#vs-code-extensions' },
            { text: 'Troubleshooting',        link: '/guide/user#troubleshooting' },
          ],
        },
      ],
      '/guide/config-generator': [
        {
          text: 'Config Generator',
          items: [
            { text: 'Generate config.toml', link: '/guide/config-generator' },
          ],
        },
      ],
    },

    socialLinks: [
      { icon: 'github', link: 'https://git.batleforc.fr/batleforc/batlehub' },
    ],

    footer: {
      message: 'Released under the MIT License.',
      copyright: 'Copyright © 2025 Max Batleforc',
    },

    search: {
      provider: 'local',
    },
  },
})
