import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { createRouter, createMemoryHistory, type Router } from "vue-router";

// The page fetches via `listPackages2`; `useAuth` (imported transitively) calls
// `me()` at module load. Mock the SDK + client so nothing touches the network.
const { listPackages2Mock, meMock, oidcRefreshMock, setConfigMock } = vi.hoisted(() => ({
  listPackages2Mock: vi.fn(),
  meMock: vi.fn(),
  oidcRefreshMock: vi.fn(),
  setConfigMock: vi.fn(),
}));

vi.mock("@/client/sdk.gen", () => ({
  listPackages2: listPackages2Mock,
  me: meMock,
  oidcRefresh: oidcRefreshMock,
}));
vi.mock("@/client/client.gen", () => ({ client: { setConfig: setConfigMock } }));

import PackageList from "./PackageList.vue";
import { useAuth } from "@/composables/useAuth";

const USER = { role: "user" as const, groups: [], has_registry_access: true };

function pkg(over: Record<string, unknown> = {}) {
  return {
    registry: "npm",
    name: "lodash",
    version: "4.17.21",
    artifact: null,
    status: { status: "available" },
    access_count: 5,
    ...over,
  };
}

function makeRouter(): Router {
  const stub = { template: "<div />" };
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/packages", component: stub },
      { path: "/packages/detail", component: stub },
    ],
  });
}

async function mountList(router: Router): Promise<VueWrapper> {
  await router.push("/packages");
  await router.isReady();
  const wrapper = mount(PackageList, { global: { plugins: [router] } });
  await flushPromises();
  return wrapper;
}

describe("PackageList (integration)", () => {
  beforeEach(() => {
    meMock.mockReset().mockResolvedValue({ data: USER });
    listPackages2Mock.mockReset();
    oidcRefreshMock.mockReset();
    setConfigMock.mockClear();
    useAuth().logout();
    localStorage.clear();
  });

  it("renders a table row per package and the total count", async () => {
    listPackages2Mock.mockResolvedValue({
      data: {
        items: [pkg(), pkg({ name: "react", version: "18.2.0" })],
        total: 2,
      },
    });
    const wrapper = await mountList(makeRouter());
    expect(wrapper.findAll("tbody tr")).toHaveLength(2);
    expect(wrapper.text()).toContain("lodash");
    expect(wrapper.text()).toContain("react");
    expect(wrapper.text()).toContain("(2)");
  });

  it("shows the API error message and no table", async () => {
    listPackages2Mock.mockResolvedValue({ error: { message: "upstream exploded" } });
    const wrapper = await mountList(makeRouter());
    expect(wrapper.text()).toContain("upstream exploded");
    expect(wrapper.find("tbody").exists()).toBe(false);
  });

  it("shows the empty state when no packages are cached", async () => {
    listPackages2Mock.mockResolvedValue({ data: { items: [], total: 0 } });
    const wrapper = await mountList(makeRouter());
    expect(wrapper.text()).toContain("No packages cached yet.");
    expect(wrapper.find("tbody").exists()).toBe(false);
  });

  it("filters rows by name via the search box", async () => {
    listPackages2Mock.mockResolvedValue({
      data: { items: [pkg(), pkg({ name: "react" })], total: 2 },
    });
    const wrapper = await mountList(makeRouter());
    await wrapper.find('input[aria-label="Filter packages"]').setValue("react");
    expect(wrapper.findAll("tbody tr")).toHaveLength(1);
    expect(wrapper.text()).toContain("react");
    expect(wrapper.text()).not.toContain("lodash");
  });

  it("shows the no-match message when the filter excludes everything", async () => {
    listPackages2Mock.mockResolvedValue({ data: { items: [pkg()], total: 1 } });
    const wrapper = await mountList(makeRouter());
    await wrapper.find('input[aria-label="Filter packages"]').setValue("nonexistent");
    expect(wrapper.text()).toContain("No packages match your filter.");
  });

  it("labels a blocked package with its reason", async () => {
    listPackages2Mock.mockResolvedValue({
      data: { items: [pkg({ status: { status: "blocked", reason: "CVE-2021-1" } })], total: 1 },
    });
    const wrapper = await mountList(makeRouter());
    expect(wrapper.text()).toContain("Blocked: CVE-2021-1");
  });

  it("navigates to the detail page (with coordinates) when a row is clicked", async () => {
    listPackages2Mock.mockResolvedValue({
      data: { items: [pkg({ artifact: "lodash-4.17.21.tgz" })], total: 1 },
    });
    const router = makeRouter();
    const wrapper = await mountList(router);
    const push = vi.spyOn(router, "push");
    await wrapper.find("tbody tr").trigger("click");
    expect(push).toHaveBeenCalledWith(
      expect.objectContaining({
        path: "/packages/detail",
        query: expect.objectContaining({
          registry: "npm",
          name: "lodash",
          version: "4.17.21",
          artifact: "lodash-4.17.21.tgz",
        }),
      }),
    );
  });

  it("re-fetches when the Refresh button is clicked", async () => {
    listPackages2Mock.mockResolvedValue({ data: { items: [pkg()], total: 1 } });
    const wrapper = await mountList(makeRouter());
    expect(listPackages2Mock).toHaveBeenCalledTimes(1);
    const refresh = wrapper.findAll("button").find((b) => b.text().includes("Refresh"))!;
    await refresh.trigger("click");
    await flushPromises();
    expect(listPackages2Mock).toHaveBeenCalledTimes(2);
  });
});
