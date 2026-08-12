/**
 * Admin navigation data: the sidebar, and the tab strip for each
 * `/admin/<section>` route.
 *
 * `label` holds an **i18n key**, not a string, matching `navigation.ts`. These
 * were English literals until the audit learned to look inside `<script>`:
 * every one of them rendered untranslated through `SectionTabs` and
 * `AdminLayout`, and the template-only scan could not see them, so the gate
 * read zero while the whole admin navigation stayed in English.
 *
 * The sidebar lives here rather than inside `AdminLayout.vue` for the same
 * reason: `catalogues.test.ts` can only prove a label resolves if it can import
 * it, and an array inside `<script setup>` cannot be imported.
 */
import {
  Bell,
  FolderKey,
  HeartPulse,
  LayoutDashboard,
  Package,
  RefreshCw,
  Shield,
} from "@lucide/vue";

export const ADMIN_SIDEBAR = [
  { to: "/admin/dashboard", label: "adminNav.dashboard", icon: LayoutDashboard },
  { to: "/admin/packages", label: "adminNav.packages", icon: Package },
  { to: "/admin/security", label: "adminNav.securityAccess", icon: Shield },
  { to: "/admin/namespaces", label: "adminNav.namespacesChannels", icon: FolderKey },
  { to: "/admin/operations", label: "adminNav.operations", icon: RefreshCw },
  { to: "/admin/observability", label: "adminNav.observability", icon: HeartPulse },
  { to: "/admin/notifications", label: "adminNav.notifications", icon: Bell },
];

export const PACKAGES_TABS = [
  { to: "/admin/packages/all", label: "adminNav.allPackages" },
  { to: "/admin/packages/bulk", label: "adminNav.bulkBlock" },
];

export const SECURITY_TABS = [
  { to: "/admin/security/users", label: "adminNav.users" },
  { to: "/admin/security/ip-blocks", label: "adminNav.ipBlocks" },
  { to: "/admin/security/access-check", label: "adminNav.accessCheck" },
];

export const NAMESPACES_TABS = [
  { to: "/admin/namespaces/team-namespaces", label: "adminNav.teamNamespaces" },
  { to: "/admin/namespaces/beta-channel", label: "adminNav.betaChannel" },
];

export const OPERATIONS_TABS = [
  { to: "/admin/operations/config-reload", label: "adminNav.configReload" },
  { to: "/admin/operations/warming", label: "adminNav.warming" },
  { to: "/admin/operations/explore-cache", label: "adminNav.exploreCache" },
];

export const OBSERVABILITY_TABS = [
  { to: "/admin/observability/health", label: "adminNav.health" },
  { to: "/admin/observability/sbom", label: "adminNav.sbomExport" },
  { to: "/admin/observability/audit-log", label: "adminNav.auditLog" },
];
