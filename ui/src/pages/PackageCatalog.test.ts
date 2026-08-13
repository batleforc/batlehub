import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";

const { explorePackagesMock, exploreUpstreamSearchMock, listRegistriesMock, statsMock } =
  vi.hoisted(() => ({
    explorePackagesMock: vi.fn(),
    exploreUpstreamSearchMock: vi.fn(),
    listRegistriesMock: vi.fn(),
    statsMock: vi.fn(),
  }));
vi.mock("@/client/sdk.gen", () => ({
  explorePackages: explorePackagesMock,
  exploreUpstreamSearch: exploreUpstreamSearchMock,
  listRegistries: listRegistriesMock,
  exploreRegistryStats: statsMock,
}));

import PackageCatalog from "./PackageCatalog.vue";
import { scopeExploreCacheTo, useExploreCache } from "@/composables/useExploreCache";

const entry = (name: string) => ({
  registry: "npm",
  name,
  latest_version: "1.0.0",
  versions: 1,
  downloads: 10,
  last_accessed: null,
  source: "proxied",
});

const listing = (names: string[], total = names.length) => ({
  data: { items: names.map(entry), total, page: 0, per_page: 20 },
});

async function mountPage() {
  const wrapper = mount(PackageCatalog, {
    global: {
      stubs: { RouterLink: { template: "<a><slot /></a>" } },
    },
  });
  await flushPromises();
  return wrapper;
}

/**
 * The page `/packages` never had a component test.
 *
 * `useExploreCache.test.ts` covered the composable in isolation — TTL, key
 * independence, `invalidate` — and never exercised it against the page, which
 * is why a store that survived logout passed a suite specifically written for
 * it (RFC 0004-bis §2.9).
 */
describe("PackageCatalog", () => {
  beforeEach(() => {
    vi.useRealTimers();
    explorePackagesMock.mockReset().mockResolvedValue(listing(["lodash"]));
    exploreUpstreamSearchMock.mockReset().mockResolvedValue({ data: { items: [] } });
    listRegistriesMock.mockReset().mockResolvedValue({ data: [{ name: "npm", type: "npm" }] });
    statsMock.mockReset().mockResolvedValue({ data: { registries: [] } });
    // A fresh viewer per test, which is also how the store is emptied between
    // them — the module-level map outlives a `mount`.
    scopeExploreCacheTo(`test-${Math.random()}`);
  });

  it("renders the rows the listing returns", async () => {
    const wrapper = await mountPage();
    expect(wrapper.text()).toContain("lodash");
  });

  it("issues no second request for a repeated search", async () => {
    const wrapper = await mountPage();
    expect(explorePackagesMock).toHaveBeenCalledTimes(1);

    // Sorting re-queries under a new key…
    await wrapper.find("select").setValue("name");
    await flushPromises();
    expect(explorePackagesMock).toHaveBeenCalledTimes(2);

    // …and going back reads the entry the first call filled.
    await wrapper.find("select").setValue("downloads");
    await flushPromises();
    expect(explorePackagesMock).toHaveBeenCalledTimes(2);
    expect(wrapper.text()).toContain("lodash");
  });

  /**
   * `packages.value = body.items` was unconditional — no sequence token, no
   * `AbortController` — and `onSortChange` is undebounced. So a slow first
   * request overwrote the table after a fast second had already filled it,
   * leaving the rows and the controls describing different queries.
   */
  it("a slow first response does not overwrite a fast second", async () => {
    let releaseSlow!: (v: unknown) => void;
    const slow = new Promise((resolve) => {
      releaseSlow = resolve;
    });

    explorePackagesMock
      .mockReturnValueOnce(slow) // initial mount: hangs
      .mockResolvedValue(listing(["fast-result"]));

    const wrapper = await mountPage();
    await wrapper.find("select").setValue("name");
    await flushPromises();
    expect(wrapper.text()).toContain("fast-result");

    // The superseded response lands last, and must not be displayed.
    releaseSlow(listing(["slow-result"]));
    await flushPromises();
    expect(wrapper.text()).toContain("fast-result");
    expect(wrapper.text()).not.toContain("slow-result");
  });

  /**
   * The late response is still *cached*: it is correct for its own key, only
   * stale for the screen. Discarding it would mean re-fetching the moment the
   * operator went back.
   */
  it("a superseded response is still written to the cache", async () => {
    let releaseSlow!: (v: unknown) => void;
    const slow = new Promise((resolve) => {
      releaseSlow = resolve;
    });
    explorePackagesMock.mockReturnValueOnce(slow).mockResolvedValue(listing(["fast-result"]));

    const wrapper = await mountPage();
    await wrapper.find("select").setValue("name");
    await flushPromises();
    releaseSlow(listing(["slow-result"]));
    await flushPromises();

    const cache = useExploreCache<{ items: { name: string }[] }>();
    // The initial mount's coordinates: default registry, page 0, "downloads".
    expect(cache.get("", 0, "downloads", "")?.items[0].name).toBe("slow-result");
  });

  it("surfaces a listing error rather than an empty table", async () => {
    explorePackagesMock.mockResolvedValue({ error: { message: "upstream exploded" } });
    const wrapper = await mountPage();
    expect(wrapper.text()).toMatch(/failed to load/i);
  });
});
