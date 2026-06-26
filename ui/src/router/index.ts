import { watch, type Ref } from "vue";
import {
  createRouter,
  createWebHistory,
  type RouteLocationNormalized,
  type RouteLocationRaw,
} from "vue-router";
import { useAuth, storeTokens } from "@/composables/useAuth";

const OIDC_STATE_KEY = "oidc_state";

type AuthState = ReturnType<typeof useAuth>;

function handleOidcCallback(to: RouteLocationNormalized): RouteLocationRaw | null {
  if (!to.query.oidc_access_token) return null;

  const incomingState = String(to.query.oidc_state ?? "");
  const expectedState = sessionStorage.getItem(OIDC_STATE_KEY) ?? "";
  sessionStorage.removeItem(OIDC_STATE_KEY);

  if (!incomingState || incomingState !== expectedState) {
    return {
      path: "/login",
      query: { error: "State mismatch — possible CSRF attack. Please try again." },
    };
  }

  const provider = to.query.oidc_provider ? String(to.query.oidc_provider) : null;
  storeTokens(
    String(to.query.oidc_access_token),
    to.query.oidc_refresh_token ? String(to.query.oidc_refresh_token) : null,
    to.query.oidc_expires_in ? Number(to.query.oidc_expires_in) : null,
    provider,
  );
  return { path: "/packages" };
}

function handleOidcError(to: RouteLocationNormalized): RouteLocationRaw | null {
  if (!to.query.oidc_error) return null;
  sessionStorage.removeItem(OIDC_STATE_KEY);
  return { path: "/login", query: { error: String(to.query.oidc_error) } };
}

function waitForIdentity(identityReady: Ref<boolean>): Promise<void> {
  if (identityReady.value) return Promise.resolve();
  return new Promise<void>((resolve) => {
    const stop = watch(identityReady, (ready) => {
      if (ready) {
        stop();
        resolve();
      }
    });
  });
}

function checkAnonymousAccess(
  to: RouteLocationNormalized,
  { identity }: Pick<AuthState, "identity">,
): RouteLocationRaw | null {
  if (
    to.path !== "/login" &&
    identity.value?.role === "anonymous" &&
    identity.value?.has_registry_access === false
  ) {
    return { path: "/login" };
  }
  return null;
}

function checkMetaGuards(
  to: RouteLocationNormalized,
  {
    isAuthenticated,
    identity,
    isAdmin,
  }: Pick<AuthState, "isAuthenticated" | "identity" | "isAdmin">,
): RouteLocationRaw | undefined {
  if (to.meta.requiresAuth && !isAuthenticated.value) {
    return { path: "/login", query: { redirect: to.fullPath } };
  }
  if (to.meta.requiresOidcAuth && !identity.value?.auth_provider) {
    return { path: "/login", query: { redirect: to.fullPath } };
  }
  if (to.meta.requiresAdmin && !isAdmin.value) {
    return { path: "/login", query: { redirect: to.fullPath } };
  }
}

export const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: "/", redirect: "/packages" },
    { path: "/login", component: () => import("@/pages/LoginPage.vue") },
    { path: "/packages", component: () => import("@/pages/PackageList.vue") },
    { path: "/packages/detail", component: () => import("@/pages/PackageDetail.vue") },
    { path: "/explore", component: () => import("@/pages/PackageExplorer.vue") },
    {
      path: "/explore/packages/:registry/:name",
      component: () => import("@/pages/ExplorePackageDetail.vue"),
    },
    { path: "/access-check", component: () => import("@/pages/AccessCheck.vue") },
    { path: "/path-mapper", component: () => import("@/pages/PathMapper.vue") },
    { path: "/setup", component: () => import("@/pages/SetupGuide.vue") },
    {
      path: "/tokens",
      component: () => import("@/pages/TokensPage.vue"),
      meta: { requiresOidcAuth: true },
    },
    {
      path: "/profile",
      component: () => import("@/pages/MyProfile.vue"),
      meta: { requiresAuth: true },
    },
    {
      path: "/my-namespace",
      component: () => import("@/pages/MyNamespace.vue"),
      meta: { requiresAuth: true },
    },
    {
      path: "/cli",
      component: () => import("@/pages/CliDownload.vue"),
      meta: { requiresAuth: true },
    },
    {
      path: "/admin",
      component: () => import("@/layouts/AdminLayout.vue"),
      meta: { requiresAdmin: true },
      children: [
        { path: "packages", component: () => import("@/pages/AdminPackages.vue") },
        { path: "packages/detail", component: () => import("@/pages/AdminPackageDetail.vue") },
        { path: "bulk", component: () => import("@/pages/AdminBulk.vue") },
        { path: "audit-log", component: () => import("@/pages/AuditLog.vue") },
        { path: "health", component: () => import("@/pages/AdminHealth.vue") },
        { path: "sbom", component: () => import("@/pages/AdminSbom.vue") },
        { path: "ip-blocks", component: () => import("@/pages/AdminIpBlocks.vue") },
        { path: "users", component: () => import("@/pages/AdminUsers.vue") },
        { path: "beta-channel", component: () => import("@/pages/AdminBetaChannel.vue") },
        { path: "team-namespaces", component: () => import("@/pages/AdminTeamNamespaces.vue") },
        { path: "config-reload", component: () => import("@/pages/AdminConfigReload.vue") },
        { path: "explore-cache", component: () => import("@/pages/AdminExploreCache.vue") },
        { path: "notifications", component: () => import("@/pages/AdminNotifications.vue") },
      ],
    },
  ],
});

router.beforeEach(async (to) => {
  const auth = useAuth();

  const oidcResult = handleOidcCallback(to);
  if (oidcResult !== null) return oidcResult;

  const oidcError = handleOidcError(to);
  if (oidcError !== null) return oidcError;

  await waitForIdentity(auth.identityReady);

  const anonResult = checkAnonymousAccess(to, auth);
  if (anonResult !== null) return anonResult;

  return checkMetaGuards(to, auth);
});

/** Generate and store a fresh OIDC state value, then return it. */
export function generateOidcState(): string {
  const state = crypto.randomUUID();
  sessionStorage.setItem(OIDC_STATE_KEY, state);
  return state;
}
