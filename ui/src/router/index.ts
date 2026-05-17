import { watch } from "vue";
import { createRouter, createWebHistory } from "vue-router";
import { useAuth, storeTokens } from "@/composables/useAuth";
import PackageList from "@/pages/PackageList.vue";
import AccessCheck from "@/pages/AccessCheck.vue";
import AdminPackages from "@/pages/AdminPackages.vue";
import AdminPackageDetail from "@/pages/AdminPackageDetail.vue";
import AuditLog from "@/pages/AuditLog.vue";
import LoginPage from "@/pages/LoginPage.vue";
import PathMapper from "@/pages/PathMapper.vue";
import SetupGuide from "@/pages/SetupGuide.vue";
import TokensPage from "@/pages/TokensPage.vue";

const OIDC_STATE_KEY = "oidc_state";

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: "/", redirect: "/packages" },
    { path: "/login", component: LoginPage },
    { path: "/packages", component: PackageList },
    { path: "/access-check", component: AccessCheck },
    { path: "/path-mapper", component: PathMapper },
    { path: "/setup", component: SetupGuide },
    {
      path: "/tokens",
      component: TokensPage,
      meta: { requiresOidcAuth: true },
    },
    {
      path: "/admin/packages",
      component: AdminPackages,
      meta: { requiresAdmin: true },
    },
    {
      path: "/admin/packages/detail",
      component: AdminPackageDetail,
      meta: { requiresAdmin: true },
    },
    {
      path: "/admin/audit-log",
      component: AuditLog,
      meta: { requiresAdmin: true },
    },
  ],
});

router.beforeEach(async (to) => {
  const { isAdmin, identity, identityReady } = useAuth();

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

    storeTokens(
      String(to.query.oidc_access_token),
      to.query.oidc_refresh_token ? String(to.query.oidc_refresh_token) : null,
      to.query.oidc_expires_in ? Number(to.query.oidc_expires_in) : null,
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

  // ── OIDC-only route guard ─────────────────────────────────────────────────
  if (to.meta.requiresOidcAuth) {
    if (!identityReady.value) {
      await new Promise<void>((resolve) => {
        const stop = watch(identityReady, (ready) => {
          if (ready) { stop(); resolve(); }
        });
      });
    }
    if (identity.value?.auth_provider !== "oidc") {
      return { path: "/login", query: { redirect: to.fullPath } };
    }
    return;
  }

  // ── Admin route guard ──────────────────────────────────────────────────────
  if (!to.meta.requiresAdmin) return;

  // On a hard page reload the me() call may still be in-flight — wait for it.
  if (!identityReady.value) {
    await new Promise<void>((resolve) => {
      const stop = watch(identityReady, (ready) => {
        if (ready) {
          stop();
          resolve();
        }
      });
    });
  }

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
