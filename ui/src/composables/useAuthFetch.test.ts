import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const { meMock, oidcRefreshMock, setConfigMock } = vi.hoisted(() => ({
  meMock: vi
    .fn()
    .mockResolvedValue({ data: { role: "anonymous", groups: [], has_registry_access: false } }),
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

import { storeTokens, useAuth } from "./useAuth";
import { useAuthFetch } from "./useAuthFetch";

describe("useAuthFetch", () => {
  beforeEach(() => {
    useAuth().logout();
    localStorage.clear();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("authHeaders() returns no Authorization header without a token", () => {
    const { authHeaders } = useAuthFetch();
    expect(authHeaders()).toEqual({});
  });

  it("authHeaders() returns a Bearer token once one is stored", () => {
    storeTokens("abc123");
    const { authHeaders } = useAuthFetch();
    expect(authHeaders()).toEqual({ Authorization: "Bearer abc123" });
  });

  it("authFetch() merges the auth header with caller-provided headers", async () => {
    storeTokens("abc123");
    const fetchMock = vi.fn().mockResolvedValue(new Response("ok"));
    vi.stubGlobal("fetch", fetchMock);

    const { authFetch } = useAuthFetch();
    await authFetch("/api/v1/test", { headers: { "X-Foo": "bar" } });

    expect(fetchMock).toHaveBeenCalledWith("/api/v1/test", {
      headers: { Authorization: "Bearer abc123", "X-Foo": "bar" },
    });
  });

  it("authFetch() works without a stored token", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("ok"));
    vi.stubGlobal("fetch", fetchMock);

    const { authFetch } = useAuthFetch();
    await authFetch("/api/v1/test");

    expect(fetchMock).toHaveBeenCalledWith("/api/v1/test", { headers: {} });
  });
});
