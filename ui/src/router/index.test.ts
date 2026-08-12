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
import { initAuth, useAuth } from "@/composables/useAuth";

// `useAuth` no longer initializes itself at import time (see `initAuth`'s doc
// comment) — call it once here, mirroring what `clientInit.ts` does in the app,
// so the singleton's initial identity fetch actually runs against the mocks above.
initAuth();

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
    await router.push("/me/profile");
    await router.isReady();
    expect(router.currentRoute.value.query.redirect).toBe("/me/profile");
  });

  it("lets an authenticated user reach a requiresAuth route", async () => {
    await setAuth(USER, "tok");
    expect(await go("/me/profile")).toBe("/me/profile");
  });

  // ── requiresOidcAuth ────────────────────────────────────────────────────────

  it("redirects a token-only (non-OIDC) user away from a requiresOidcAuth route", async () => {
    await setAuth(USER, "tok"); // authenticated but no auth_provider
    expect(await go("/me/tokens")).toBe("/login");
  });

  it("lets an OIDC-authenticated user reach a requiresOidcAuth route", async () => {
    await setAuth(USER_OIDC, "tok");
    expect(await go("/me/tokens")).toBe("/me/tokens");
  });

  // ── requiresAdmin ─────────────────────────────────────────────────────────

  it("redirects a non-admin away from an admin route", async () => {
    await setAuth(USER, "tok");
    expect(await go("/admin/observability/health")).toBe("/login");
  });

  it("lets an admin reach an admin route", async () => {
    await setAuth(ADMIN, "tok");
    expect(await go("/admin/observability/health")).toBe("/admin/observability/health");
  });

  // ── Admin nav regrouping: old flat paths redirect to their new section ─────

  it.each([
    ["/admin/packages", "/admin/packages/all"],
    ["/admin/bulk", "/admin/packages/bulk"],
    ["/admin/users", "/admin/security/users"],
    ["/admin/ip-blocks", "/admin/security/ip-blocks"],
    ["/admin/access-check", "/admin/security/access-check"],
    ["/admin/team-namespaces", "/admin/namespaces/team-namespaces"],
    ["/admin/beta-channel", "/admin/namespaces/beta-channel"],
    ["/admin/config-reload", "/admin/operations/config-reload"],
    ["/admin/warming", "/admin/operations/warming"],
    ["/admin/explore-cache", "/admin/operations/explore-cache"],
    ["/admin/health", "/admin/observability/health"],
    ["/admin/sbom", "/admin/observability/sbom"],
    ["/admin/audit-log", "/admin/observability/audit-log"],
  ])("redirects the old admin path %s to %s", async (oldPath, newPath) => {
    await setAuth(ADMIN, "tok");
    expect(await go(oldPath)).toBe(newPath);
  });

  it.each([
    ["/admin", "/admin/dashboard"],
    ["/admin/security", "/admin/security/users"],
    ["/admin/namespaces", "/admin/namespaces/team-namespaces"],
    ["/admin/operations", "/admin/operations/config-reload"],
    ["/admin/observability", "/admin/observability/health"],
  ])("redirects the section base path %s to its first tab %s", async (basePath, firstTab) => {
    await setAuth(ADMIN, "tok");
    expect(await go(basePath)).toBe(firstTab);
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
      "/packages/npm1/left-pad",
      "/setup",
      "/",
      "/tools/access-check",
      "/tools/url-mapper",
      "/me/profile",
      "/me/tokens",
      "/me/namespace",
      "/me/cli",
      "/admin/dashboard",
      "/admin/packages/all",
      "/admin/packages/bulk",
      "/admin/security/users",
      "/admin/security/ip-blocks",
      "/admin/security/access-check",
      "/admin/namespaces/team-namespaces",
      "/admin/namespaces/beta-channel",
      "/admin/operations/config-reload",
      "/admin/operations/warming",
      "/admin/operations/explore-cache",
      "/admin/observability/health",
      "/admin/observability/sbom",
      "/admin/observability/audit-log",
      "/admin/notifications",
    ];

    for (const p of paths) {
      expect(await go(p)).toBe(p);
    }

    // "/" is a real surface now, not a redirect: it adapts to the viewer and to
    // whether the instance has been configured yet (RFC 0003 §4.3).
    expect(await go("/packages")).toBe("/packages");
  }, 15000);
});

// ── Information architecture (RFC 0003 §4.2, §9) ────────────────────────────

describe("information architecture", () => {
  /* Own reset: the suite above owns its beforeEach, and without one here each
     test inherits wherever the last one finished — where push() to the current
     path is a duplicate navigation that never runs a guard. */
  beforeEach(async () => {
    meMock.mockReset().mockResolvedValue({ data: ANON });
    useAuth().logout();
    localStorage.clear();
    sessionStorage.clear();
    await setAuth(ANON, "");
    await router.replace("/login");
  });

  /**
   * Deep links in docs/, bookmarks and CI scripts point at the old paths, so a
   * redirect that drops one is a review blocker. These assert the whole §9 table,
   * not a sample — the table is data, and data is exactly what rots silently.
   */
  describe("legacy paths keep resolving", () => {
    const ACCOUNT: [string, string][] = [
      ["/profile", "/me/profile"],
      ["/my-namespace", "/me/namespace"],
      ["/cli", "/me/cli"],
    ];

    it.each(ACCOUNT)("%s lands on %s", async (from, to) => {
      await setAuth(USER, "tok");
      expect(await go(from)).toBe(to);
    });

    const ADMIN_ALIASES: [string, string][] = [
      ["/admin/bulk", "/admin/packages/bulk"],
      ["/admin/users", "/admin/security/users"],
      ["/admin/ip-blocks", "/admin/security/ip-blocks"],
      ["/admin/access-check", "/admin/security/access-check"],
      ["/admin/team-namespaces", "/admin/namespaces/team-namespaces"],
      ["/admin/beta-channel", "/admin/namespaces/beta-channel"],
      ["/admin/config-reload", "/admin/operations/config-reload"],
      ["/admin/warming", "/admin/operations/warming"],
      ["/admin/explore-cache", "/admin/operations/explore-cache"],
      ["/admin/health", "/admin/observability/health"],
      ["/admin/sbom", "/admin/observability/sbom"],
      ["/admin/audit-log", "/admin/observability/audit-log"],
    ];

    it.each(ADMIN_ALIASES)("%s lands on %s", async (from, to) => {
      await setAuth(ADMIN, "tok");
      expect(await go(from)).toBe(to);
    });

    const DIAGNOSTICS: [string, string][] = [
      ["/access-check", "/tools/access-check"],
      ["/path-mapper", "/tools/url-mapper"],
    ];

    it.each(DIAGNOSTICS)("%s lands on %s", async (from, to) => {
      await setAuth(ANON, "");
      expect(await go(from)).toBe(to);
    });

    /** A bookmark carries its query; losing it silently changes what you see. */
    it("preserves the query string across a redirect", async () => {
      await setAuth(ADMIN, "tok");
      await router.push("/admin/warming?registry=npm1");
      await router.isReady();
      expect(router.currentRoute.value.path).toBe("/admin/operations/warming");
      expect(router.currentRoute.value.query.registry).toBe("npm1");
    });

    const SECTIONS: [string, string][] = [
      ["/me", "/me/profile"],
      ["/tools", "/tools/access-check"],
      ["/admin", "/admin/dashboard"],
      ["/admin/packages", "/admin/packages/all"],
      ["/admin/security", "/admin/security/users"],
      ["/admin/namespaces", "/admin/namespaces/team-namespaces"],
      ["/admin/operations", "/admin/operations/config-reload"],
      ["/admin/observability", "/admin/observability/health"],
    ];

    it.each(SECTIONS)("section index %s lands on %s", async (from, to) => {
      await setAuth(ADMIN, "tok");
      expect(await go(from)).toBe(to);
    });
  });

  describe("the hubs keep their guards", () => {
    /** Moving a route must not weaken what protected it. */
    it("sends an unauthenticated visitor from an account tab to /login", async () => {
      await setAuth(ANON, "");
      expect(await go("/me/namespace")).toBe("/login");
    });

    it("keeps /me/tokens OIDC-only after the move", async () => {
      await setAuth(USER, "tok"); // authenticated, but no auth_provider
      expect(await go("/me/tokens")).toBe("/login");
    });

    it("lets an OIDC user reach /me/tokens", async () => {
      await setAuth(USER_OIDC, "tok");
      expect(await go("/me/tokens")).toBe("/me/tokens");
    });

    it("keeps the admin guard on a legacy admin alias", async () => {
      await setAuth(USER, "tok");
      expect(await go("/admin/users")).toBe("/login");
    });

    /** Diagnostics are public; a denied request is where they get linked from. */
    it("lets an anonymous visitor use the diagnostics hub", async () => {
      await setAuth(ANON, "");
      expect(await go("/tools/url-mapper")).toBe("/tools/url-mapper");
    });
  });

  describe("the catalog has one address", () => {
    /**
     * A package used to have two URLs — `/packages/detail?registry=&name=` and
     * `/explore/packages/:registry/:name` — and only one survived a copy-paste.
     * Both now resolve to the canonical path form (RFC 0003 §9).
     */
    it("sends /explore to the merged catalog", async () => {
      await setAuth(ANON, "");
      expect(await go("/explore")).toBe("/packages");
    });

    it("converts the explore detail URL to the canonical one", async () => {
      await setAuth(ANON, "");
      expect(await go("/explore/packages/npm1/left-pad")).toBe("/packages/npm1/left-pad");
    });

    it("converts the query-param detail URL to the canonical one", async () => {
      await setAuth(ANON, "");
      expect(await go("/packages/detail?registry=npm1&name=left-pad")).toBe(
        "/packages/npm1/left-pad",
      );
    });

    it("converts the admin detail URL to the same canonical page", async () => {
      await setAuth(ADMIN, "tok");
      expect(await go("/admin/packages/detail?registry=npm1&name=left-pad")).toBe(
        "/packages/npm1/left-pad",
      );
    });

    /** Version and artifact select *within* a package, so they stay as query. */
    it("keeps the version and artifact query across the conversion", async () => {
      await setAuth(ANON, "");
      await router.push(
        "/packages/detail?registry=npm1&name=left-pad&version=1.3.0&artifact=x.tgz",
      );
      await router.isReady();
      expect(router.currentRoute.value.path).toBe("/packages/npm1/left-pad");
      expect(router.currentRoute.value.query.version).toBe("1.3.0");
      expect(router.currentRoute.value.query.artifact).toBe("x.tgz");
      expect(router.currentRoute.value.query.registry).toBeUndefined();
    });

    /** Without both parts we cannot name a package; the catalog is honest. */
    it("falls back to the catalog when the old URL names no package", async () => {
      await setAuth(ANON, "");
      expect(await go("/packages/detail")).toBe("/packages");
    });

    it("round-trips a scoped npm name", async () => {
      await setAuth(ANON, "");
      expect(await go("/packages/detail?registry=npm1&name=%40scope%2Fpkg")).toBe(
        "/packages/npm1/%40scope%2Fpkg",
      );
    });
  });

  describe("the home route", () => {
    it("no longer redirects — it renders for whoever asks", async () => {
      await setAuth(ANON, "");
      expect(await go("/")).toBe("/");
    });

    it("is still closed to an anonymous viewer with no registry access", async () => {
      await setAuth(ANON_NO_ACCESS, "");
      expect(await go("/")).toBe("/login");
    });
  });
});
