import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";

// --- Mocks (must be hoisted before imports) ----------------------------------

const { meMock, setConfigMock, oidcRefreshMock } = vi.hoisted(() => ({
  meMock: vi.fn(),
  setConfigMock: vi.fn(),
  oidcRefreshMock: vi.fn(),
}));

// useAuth uses the SDK client transitively.
vi.mock("@/client/sdk.gen", () => ({
  me: meMock,
  oidcRefresh: oidcRefreshMock,
}));
vi.mock("@/client/client.gen", () => ({ client: { setConfig: setConfigMock } }));

// Global fetch mock — AdminUsers.vue uses authFetch which calls window.fetch.
const fetchMock = vi.fn();
vi.stubGlobal("fetch", fetchMock);

import AdminUsers from "./AdminUsers.vue";
import { useAuth } from "@/composables/useAuth";

// --------------------------------------------------------------------------

const ALICE: import("./AdminUsers.vue").default = undefined as unknown as typeof AdminUsers;

function blockedUser(over: Record<string, unknown> = {}) {
  return {
    user_id: "alice",
    blocked_at: "2026-01-01T00:00:00Z",
    blocked_by: "admin",
    reason: "spammer",
    ...over,
  };
}

function okJson(data: unknown): Response {
  return {
    ok: true,
    status: 200,
    json: async () => data,
    headers: new Headers(),
  } as unknown as Response;
}

function noContent(): Response {
  return { ok: true, status: 204, json: async () => null } as unknown as Response;
}

function errJson(status: number, body: Record<string, string>): Response {
  return {
    ok: false,
    status,
    json: async () => body,
  } as unknown as Response;
}

async function mountPage(): Promise<VueWrapper> {
  const wrapper = mount(AdminUsers, { attachTo: document.body });
  await flushPromises();
  return wrapper;
}

describe("AdminUsers", () => {
  beforeEach(() => {
    fetchMock.mockReset();
    meMock.mockReset().mockResolvedValue({ data: { role: "admin" } });
    oidcRefreshMock.mockReset();
    setConfigMock.mockClear();
    useAuth().logout();
    localStorage.clear();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  // ── List ───────────────────────────────────────────────────────────────────

  it("shows 'No users are currently blocked' when the list is empty", async () => {
    fetchMock.mockResolvedValueOnce(okJson([]));
    const wrapper = await mountPage();
    expect(wrapper.text()).toContain("No users are currently blocked.");
  });

  it("renders a row per blocked user", async () => {
    fetchMock.mockResolvedValueOnce(okJson([blockedUser(), blockedUser({ user_id: "bob" })]));
    const wrapper = await mountPage();
    const rows = wrapper.findAll("tbody tr");
    expect(rows).toHaveLength(2);
    expect(wrapper.text()).toContain("alice");
    expect(wrapper.text()).toContain("bob");
  });

  it("shows the blocked-by and reason columns", async () => {
    fetchMock.mockResolvedValueOnce(okJson([blockedUser()]));
    const wrapper = await mountPage();
    expect(wrapper.text()).toContain("admin");
    expect(wrapper.text()).toContain("spammer");
  });

  it("shows '—' when reason is null", async () => {
    fetchMock.mockResolvedValueOnce(okJson([blockedUser({ reason: null })]));
    const wrapper = await mountPage();
    expect(wrapper.text()).toContain("—");
  });

  it("shows an error message when the list fetch fails", async () => {
    fetchMock.mockResolvedValueOnce({ ok: false, status: 500 } as Response);
    const wrapper = await mountPage();
    expect(wrapper.text()).toContain("HTTP 500");
  });

  // ── Block ──────────────────────────────────────────────────────────────────

  it("opens the block dialog when 'Block User' is clicked", async () => {
    fetchMock.mockResolvedValueOnce(okJson([]));
    const wrapper = await mountPage();
    const btn = wrapper.findAll("button").find((b) => b.text().includes("Block User"))!;
    await btn.trigger("click");
    expect(wrapper.text()).toContain("Block user");
  });

  it("calls POST /api/v1/admin/users/{id}/block on submit", async () => {
    fetchMock
      .mockResolvedValueOnce(okJson([]))   // initial list load
      .mockResolvedValueOnce(noContent())  // block request
      .mockResolvedValueOnce(okJson([]));  // list refresh after block

    const wrapper = await mountPage();

    // Drive the block directly through the component's VM (dialog uses a portal
    // rendered outside the wrapper, so DOM selectors won't find it).
    const vm = wrapper.vm as unknown as {
      blockForm: { user_id: string; reason: string };
      submitBlock: () => Promise<void>;
    };
    vm.blockForm.user_id = "charlie";
    vm.blockForm.reason = "";
    await vm.submitBlock();
    await flushPromises();

    expect(fetchMock).toHaveBeenCalledWith(
      expect.stringContaining("/api/v1/admin/users/charlie/block"),
      expect.objectContaining({ method: "POST" }),
    );
  });

  // ── Unblock ────────────────────────────────────────────────────────────────

  it("calls DELETE /api/v1/admin/users/{id}/block on unblock confirm", async () => {
    fetchMock
      .mockResolvedValueOnce(okJson([blockedUser()]))  // initial list
      .mockResolvedValueOnce(noContent())              // unblock request
      .mockResolvedValueOnce(okJson([]));              // list refresh

    const wrapper = await mountPage();

    const vm = wrapper.vm as unknown as {
      unblockTarget: string | null;
      confirmUnblock: () => Promise<void>;
    };
    vm.unblockTarget = "alice";
    await vm.confirmUnblock();
    await flushPromises();

    expect(fetchMock).toHaveBeenCalledWith(
      expect.stringContaining("/api/v1/admin/users/alice/block"),
      expect.objectContaining({ method: "DELETE" }),
    );
  });

  it("shows unblock error when DELETE fails", async () => {
    fetchMock
      .mockResolvedValueOnce(okJson([blockedUser()]))
      .mockResolvedValueOnce({ ok: false, status: 500, json: async () => ({}) } as Response);

    const wrapper = await mountPage();

    const vm = wrapper.vm as unknown as {
      unblockTarget: string | null;
      unblockError: string | null;
      confirmUnblock: () => Promise<void>;
    };
    vm.unblockTarget = "alice";
    await vm.confirmUnblock();
    await flushPromises();

    expect(vm.unblockError).toContain("HTTP 500");
  });
});
