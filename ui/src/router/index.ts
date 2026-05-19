import { watch } from "vue";
import { createRouter, createWebHistory } from "vue-router";
import { useAuth, storeTokens } from "@/composables/useAuth";
import PackageList from "@/pages/PackageList.vue";
import PackageDetail from "@/pages/PackageDetail.vue";
import AccessCheck from "@/pages/AccessCheck.vue";
import AdminPackages from "@/pages/AdminPackages.vue";
import AdminPackageDetail from "@/pages/AdminPackageDetail.vue";
import AdminBulk from "@/pages/AdminBulk.vue";
import AdminHealth from "@/pages/AdminHealth.vue";
import AuditLog from "@/pages/AuditLog.vue";
import LoginPage from "@/pages/LoginPage.vue";
import PathMapper from "@/pages/PathMapper.vue";
import SetupGuide from "@/pages/SetupGuide.vue";
import TokensPage from "@/pages/TokensPage.vue";
import MyProfile from "@/pages/MyProfile.vue";
import AdminLayout from "@/layouts/AdminLayout.vue";

const OIDC_STATE_KEY = "oidc_state";

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: "/", redirect: "/packages" },
    { path: "/login", component: LoginPage },
    { path: "/packages", component: PackageList },
    { path: "/packages/detail", component: PackageDetail },
    { path: "/access-check", component: AccessCheck },
    { path: "/path-mapper", component: PathMapper },
    { path: "/setup", component: SetupGuide },
    {
      path: "/tokens",
      component: TokensPage,
      meta: { requiresOidcAuth: true },
    },
    {
      path: "/profile",
      component: MyProfile,
      meta: { requiresAuth: true },
    },
    {
      path: "/admin",
      component: AdminLayout,
      meta: { requiresAdmin: true },
      children: [
        { path: "packages",        component: AdminPackages },
        { path: "packages/detail", component: AdminPackageDetail },
        { path: "bulk",            component: AdminBulk },
        { path: "audit-log",       component: AuditLog },
        { path: "health",          component: AdminHealth },
      ],
    },
  ],
});

router.beforeEach(async (to) => {
  const { isAdmin, isAuthenticated, identity, identityReady } = useAuth();

  // ── OIDC callback: tokens arrive via query params on "/" ───────────────────
  if (to.query.oidc_access_token) {
    const incomingState = String(to.query.oidc_state ?? "");
    const expectedState = sessionStorage.getItem(OIDC_STATE_KEY) ?? "";

    // Validate state to prevent CSRF / open-redirect abuse.
    if (!incomingState || incomingState !== expectedState) {
      sessionStorage.removeItem(OIDC_STATE_KEY);
      return {
        path: "/login",
        query: { error: "State mismatch — possible CSRF attack. Please try again." },
      };
    }

    sessionStorage.removeItem(OIDC_STATE_KEY);

    const provider = to.query.oidc_provider ? String(to.query.oidc_provider) : null;

    storeTokens(
      String(to.query.oidc_access_token),
      to.query.oidc_refresh_token ? String(to.query.oidc_refresh_token) : null,
      to.query.oidc_expires_in ? Number(to.query.oidc_expires_in) : null,
      provider,
    );

    return { path: "/packages" };
  }

  // ── OIDC error forwarded from backend ─────────────────────────────────────
  if (to.query.oidc_error) {
    sessionStorage.removeItem(OIDC_STATE_KEY);
    return {
      path: "/login",
      query: { error: String(to.query.oidc_error) },
    };
  }

  // ── Wait for identity (needed by all subsequent guards) ───────────────────
  if (!identityReady.value) {
    await new Promise<void>((resolve) => {
      const stop = watch(identityReady, (ready) => {
        if (ready) { stop(); resolve(); }
      });
    });
  }

  // ── Force login when anonymous has no access to any registry ──────────────
  if (
    to.path !== "/login" &&
    identity.value?.role === "anonymous" &&
    identity.value?.has_registry_access === false
  ) {
    return { path: "/login" };
  }

  // ── Authenticated-user route guard ───────────────────────────────────────
  if (to.meta.requiresAuth) {
    if (!isAuthenticated.value) {
      return { path: "/login", query: { redirect: to.fullPath } };
    }
    return;
  }

  // ── OIDC-only route guard (tokens page requires any OIDC provider) ────────
  if (to.meta.requiresOidcAuth) {
    // Any non-null auth_provider means the session came through OIDC or Kubernetes.
    if (!identity.value?.auth_provider) {
      return { path: "/login", query: { redirect: to.fullPath } };
    }
    return;
  }

  // ── Admin route guard ──────────────────────────────────────────────────────
  if (!to.meta.requiresAdmin) return;

  if (!isAdmin.value) {
    return { path: "/login", query: { redirect: to.fullPath } };
  }
});

/** Generate and store a fresh OIDC state value, then return it. */
export function generateOidcState(): string {
  const state = crypto.randomUUID();
  sessionStorage.setItem(OIDC_STATE_KEY, state);
  return state;
}
