/** Tab strips shared by the admin pages grouped under each `/admin/<section>` route. */

export const PACKAGES_TABS = [
  { to: "/admin/packages/all", label: "All Packages" },
  { to: "/admin/packages/bulk", label: "Bulk Import" },
];

export const SECURITY_TABS = [
  { to: "/admin/security/users", label: "Users" },
  { to: "/admin/security/ip-blocks", label: "IP Blocks" },
  { to: "/admin/security/access-check", label: "Access Check" },
];

export const NAMESPACES_TABS = [
  { to: "/admin/namespaces/team-namespaces", label: "Team Namespaces" },
  { to: "/admin/namespaces/beta-channel", label: "Beta Channel" },
];

export const OPERATIONS_TABS = [
  { to: "/admin/operations/config-reload", label: "Config Reload" },
  { to: "/admin/operations/warming", label: "Warming" },
  { to: "/admin/operations/explore-cache", label: "Explore Cache" },
];

export const OBSERVABILITY_TABS = [
  { to: "/admin/observability/health", label: "Health" },
  { to: "/admin/observability/sbom", label: "SBOM Export" },
  { to: "/admin/observability/audit-log", label: "Audit Log" },
];
