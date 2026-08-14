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

const entry = (name: string, over: Record<string, unknown> = {}) => ({
  registry: "npm",
  name,
  latest_version: "1.0.0",
  versions: 1,
  downloads: 10,
  last_accessed: null,
  source: "proxied",
  state: "cached",
  cached_versions: 1,
  cached_bytes: 4096,
  last_fetched_at: null,
  newest_version: "1.0.0",
  has_blocked: false,
  has_yanked: false,
  ...over,
});

const listing = (names: string[], total = names.length) => ({
  data: { items: names.map((n) => entry(n)), total, page: 0, per_page: 20 },
});

/** A listing of one row whose state the server graded. */
const graded = (state: string, over: Record<string, unknown> = {}) => ({
  data: {
    items: [entry("graded-pkg", { state, ...over })],
    total: 1,
    page: 0,
    per_page: 20,
  },
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

    // …and going back to the default order reads the entry the first call
    // filled. `fetched`, not `downloads`: the catalog is ordered by last fetch,
    // which is what its caption claims and what the proof settles on.
    await wrapper.find("select").setValue("fetched");
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
    // The initial mount's coordinates: default registry, page 0, "fetched".
    expect(cache.get("", 0, "fetched", "")?.items[0].name).toBe("slow-result");
  });

  it("surfaces a listing error rather than an empty table", async () => {
    explorePackagesMock.mockResolvedValue({ error: { message: "upstream exploded" } });
    const wrapper = await mountPage();
    expect(wrapper.text()).toMatch(/failed to load/i);
  });
});

/**
 * Resolution as state — DESIGN.md's organising idea, and the one this page is
 * the proving surface for. The grading is the server's; what is checked here is
 * that the page *renders what it was told* rather than re-deciding it, which is
 * how the old table ended up showing two states for a world that has six.
 */
describe("PackageCatalog resolution column", () => {
  beforeEach(() => {
    vi.useRealTimers();
    explorePackagesMock.mockReset().mockResolvedValue(listing(["lodash"]));
    exploreUpstreamSearchMock.mockReset().mockResolvedValue({ data: { items: [] } });
    listRegistriesMock.mockReset().mockResolvedValue({ data: [{ name: "npm", type: "npm" }] });
    statsMock.mockReset().mockResolvedValue({ data: { registries: [] } });
    scopeExploreCacheTo(`test-${Math.random()}`);
  });

  /**
   * The fine 3×3 is "held and verified"; the coarse 2×2 is everything else.
   * Asserted on the cell count rather than on a class, because the count *is*
   * the distinction — a 2×2 rendered with the fine class would still read as
   * coarse to anyone looking at it.
   */
  it("draws a fine matrix for what is held and a coarse one for what is not", async () => {
    explorePackagesMock.mockResolvedValue(graded("cached"));
    let matrix = (await mountPage()).find("[data-testid='resolution-matrix']");
    expect(matrix.findAll("i")).toHaveLength(9);

    scopeExploreCacheTo(`test-${Math.random()}`);
    explorePackagesMock.mockResolvedValue(graded("pending"));
    matrix = (await mountPage()).find("[data-testid='resolution-matrix']");
    expect(matrix.findAll("i")).toHaveLength(4);
  });

  it("names each of the six states the server can return", async () => {
    const expected: Record<string, RegExp> = {
      cached: /cached/i,
      stale: /stale/i,
      held: /held/i,
      pending: /pending/i,
      yanked: /yanked/i,
      blocked: /blocked/i,
    };
    for (const [state, pattern] of Object.entries(expected)) {
      scopeExploreCacheTo(`test-${state}-${Math.random()}`);
      explorePackagesMock.mockResolvedValue(graded(state));
      const wrapper = await mountPage();
      expect(wrapper.find("[data-state]").attributes("data-state")).toBe(state);
      expect(wrapper.text()).toMatch(pattern);
    }
  });

  /**
   * An unknown state must not render as an unlabelled mark or crash the row. A
   * server that grows a seventh state should degrade to "we do not hold this",
   * which is the safe reading.
   */
  it("falls back to pending for a state it does not recognise", async () => {
    explorePackagesMock.mockResolvedValue(graded("quantum"));
    const wrapper = await mountPage();
    expect(wrapper.find("[data-state]").attributes("data-state")).toBe("pending");
  });

  /**
   * The proof states a denial's rule in its own row, tied to the package by
   * `aria-describedby`. Without the tie it is a paragraph a screen reader meets
   * after the row and has to relate by proximity.
   */
  it("gives a refused package a note, tied to the package cell", async () => {
    explorePackagesMock.mockResolvedValue(graded("blocked"));
    const wrapper = await mountPage();

    const described = wrapper.find("[aria-describedby]");
    expect(described.exists()).toBe(true);
    const noteId = described.attributes("aria-describedby")!;
    const note = wrapper.find(`#${CSS.escape(noteId)}`);
    expect(note.exists()).toBe(true);
    expect(note.text()).toMatch(/administrator/i);
  });

  it("leaves a package that is not refused without a note", async () => {
    explorePackagesMock.mockResolvedValue(graded("cached"));
    const wrapper = await mountPage();
    expect(wrapper.find("[aria-describedby]").exists()).toBe(false);
  });

  /**
   * `cached_bytes: null` is "we never recorded a size", which is a different
   * fact from zero — rendering it as `0 B` would claim we hold an empty file.
   */
  it("renders an unknown size as a dash rather than as zero bytes", async () => {
    explorePackagesMock.mockResolvedValue(graded("cached", { cached_bytes: null }));
    const wrapper = await mountPage();
    expect(wrapper.text()).not.toMatch(/0 B/);
  });

  /** The caption is what tells two identically-shaped tables apart. */
  it("states the row count and the ordering in the caption", async () => {
    explorePackagesMock.mockResolvedValue(listing(["a", "b"], 2));
    const wrapper = await mountPage();
    expect(wrapper.find("caption").text()).toMatch(/2 packages/i);
    expect(wrapper.find("caption").text()).toMatch(/last fetch/i);
  });
});
