import { describe, it, expect, vi, beforeEach } from "vitest";
import { nextTick } from "vue";
import type { MeResponse } from "@/client/types.gen";

// The router transitively imports `useAuth`, which calls the generated SDK's
// `me()` at module load. Mock the SDK + client so no real `fetch` runs and we
// can drive the identity the guards see.
const { meMock, oidcRefreshMock, setConfigMock } = vi.hoisted(() => ({
  meMock: vi.fn(),
  oidcRefreshMock: vi.fn(),
  setConfigMock: vi.fn(),
}));

vi.mock("@/client/sdk.gen", () => ({
  me: meMock,
  oidcRefresh: oidcRefreshMock,
}));
vi.mock("@/client/client.gen", () => ({
  client: { setConfig: setConfigMock },
}));

import { router, generateOidcState } from "./index";
import { useAuth } from "@/composables/useAuth";

const ANON: MeResponse = { role: "anonymous", groups: [], has_registry_access: true };
const ANON_NO_ACCESS: MeResponse = { role: "anonymous", groups: [], has_registry_access: false };
const USER: MeResponse = { role: "user", groups: [], has_registry_access: true };
const USER_OIDC: MeResponse = {
  role: "user",
  groups: [],
  has_registry_access: true,
  auth_provider: "keycloak",
};
const ADMIN: MeResponse = { role: "admin", groups: ["admins"], has_registry_access: true };

/**
 * Drive the singleton `useAuth` state to a deterministic identity. Setting the
 * access token triggers an async identity refetch (the `watch(token)` in
 * `useAuth`), so we point `me()` at the same identity and wait for it to settle,
 * then re-assert — leaving no pending refetch to clobber the state mid-navigation.
 */
async function setAuth(identity: MeResponse | null, token: string): Promise<void> {
  const auth = useAuth();
  meMock.mockResolvedValue({ data: identity });
  auth.token.value = token;
  await nextTick();
  await vi.waitFor(() => expect(auth.identityReady.value).toBe(true));
  auth.identity.value = identity;
  auth.identityReady.value = true;
}

/** Navigate and resolve to the final path (after any guard redirect). */
async function go(to: string | { path: string; query?: Record<string, string> }): Promise<string> {
  await router.push(to);
  await router.isReady();
  return router.currentRoute.value.path;
}

describe("router navigation guards (integration)", () => {
  beforeEach(async () => {
    meMock.mockReset().mockResolvedValue({ data: ANON });
    oidcRefreshMock.mockReset();
    setConfigMock.mockClear();
    useAuth().logout();
    localStorage.clear();
    sessionStorage.clear();
    // Settle the refetch kicked off by logout() clearing the token.
    await setAuth(ANON, "");
    // Neutral starting point: `/login` is always reachable.
    await router.replace("/login");
  });

  // ── Public access & anonymous gating ──────────────────────────────────────

  it("lets an anonymous user with registry access reach a public page", async () => {
    await setAuth(ANON, "");
    expect(await go("/packages")).toBe("/packages");
  });

  it("redirects an anonymous user without registry access to /login", async () => {
    await setAuth(ANON_NO_ACCESS, "");
    expect(await go("/packages")).toBe("/login");
  });

  it("never traps the anonymous-no-access user on /login itself", async () => {
    await setAuth(ANON_NO_ACCESS, "");
    expect(await go("/login")).toBe("/login");
  });

  // ── requiresAuth ──────────────────────────────────────────────────────────

  it("redirects an unauthenticated user away from a requiresAuth route", async () => {
    await setAuth(ANON, "");
    expect(await go("/profile")).toBe("/login");
  });

  it("preserves the original destination as a ?redirect query", async () => {
    await setAuth(ANON, "");
    await router.push("/profile");
    await router.isReady();
    expect(router.currentRoute.value.query.redirect).toBe("/profile");
  });

  it("lets an authenticated user reach a requiresAuth route", async () => {
    await setAuth(USER, "tok");
    expect(await go("/profile")).toBe("/profile");
  });

  // ── requiresOidcAuth ────────────────────────────────────────────────────────

  it("redirects a token-only (non-OIDC) user away from a requiresOidcAuth route", async () => {
    await setAuth(USER, "tok"); // authenticated but no auth_provider
    expect(await go("/tokens")).toBe("/login");
  });

  it("lets an OIDC-authenticated user reach a requiresOidcAuth route", async () => {
    await setAuth(USER_OIDC, "tok");
    expect(await go("/tokens")).toBe("/tokens");
  });

  // ── requiresAdmin ─────────────────────────────────────────────────────────

  it("redirects a non-admin away from an admin route", async () => {
    await setAuth(USER, "tok");
    expect(await go("/admin/health")).toBe("/login");
  });

  it("lets an admin reach an admin route", async () => {
    await setAuth(ADMIN, "tok");
    expect(await go("/admin/health")).toBe("/admin/health");
  });

  // ── OIDC callback handling ──────────────────────────────────────────────────

  it("accepts an OIDC callback whose state matches and lands on /packages", async () => {
    await setAuth(ANON, "");
    const state = generateOidcState();
    const path = await go({
      path: "/login",
      query: {
        oidc_access_token: "access-xyz",
        oidc_refresh_token: "refresh-xyz",
        oidc_expires_in: "3600",
        oidc_state: state,
        oidc_provider: "keycloak",
      },
    });
    expect(path).toBe("/packages");
    expect(localStorage.getItem("batlehub_access_token")).toBe("access-xyz");
    expect(localStorage.getItem("batlehub_refresh_token")).toBe("refresh-xyz");
  });

  it("rejects an OIDC callback whose state does not match (CSRF) and surfaces an error", async () => {
    await setAuth(ANON, "");
    generateOidcState(); // a different expected state is stored
    const path = await go({
      path: "/login",
      query: { oidc_access_token: "access-xyz", oidc_state: "forged-state" },
    });
    expect(path).toBe("/login");
    expect(String(router.currentRoute.value.query.error)).toMatch(/CSRF/i);
    // No tokens were stored from the forged callback.
    expect(localStorage.getItem("batlehub_access_token")).toBeNull();
  });

  it("rejects an OIDC callback with no expected state at all", async () => {
    await setAuth(ANON, "");
    sessionStorage.clear(); // nothing was generated → no expected state
    const path = await go({
      path: "/login",
      query: { oidc_access_token: "access-xyz", oidc_state: "whatever" },
    });
    expect(path).toBe("/login");
    expect(localStorage.getItem("batlehub_access_token")).toBeNull();
  });

  it("surfaces an upstream oidc_error on the login page", async () => {
    await setAuth(ANON, "");
    const path = await go({
      path: "/login",
      query: { oidc_error: "access_denied" },
    });
    expect(path).toBe("/login");
    expect(router.currentRoute.value.query.error).toBe("access_denied");
  });

  // ── Every registered route resolves for an admin ────────────────────────────
  // Exercises each route record (and its lazy component loader), catching a
  // mis-registered route or a guard that wrongly blocks a privileged user.

  it("resolves every registered route for an OIDC admin", async () => {
    const ADMIN_OIDC: MeResponse = { ...ADMIN, auth_provider: "keycloak" };
    await setAuth(ADMIN_OIDC, "tok");

    const paths = [
      "/packages",
      "/packages/detail",
      "/explore",
      "/explore/packages/npm/lodash",
      "/access-check",
      "/path-mapper",
      "/setup",
      "/tokens",
      "/profile",
      "/my-namespace",
      "/cli",
      "/admin/packages",
      "/admin/packages/detail",
      "/admin/bulk",
      "/admin/audit-log",
      "/admin/health",
      "/admin/sbom",
      "/admin/ip-blocks",
      "/admin/beta-channel",
      "/admin/team-namespaces",
      "/admin/config-reload",
      "/admin/explore-cache",
      "/admin/notifications",
    ];

    for (const p of paths) {
      expect(await go(p)).toBe(p);
    }

    // "/" redirects to "/packages".
    expect(await go("/")).toBe("/packages");
  });
});
