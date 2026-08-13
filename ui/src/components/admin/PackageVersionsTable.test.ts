import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import { createRouter, createMemoryHistory } from "vue-router";
import type { PackageVersionDetail } from "@/client/types.gen";

const {
  blockPackageMock,
  unblockPackageMock,
  bulkBlockPackagesMock,
  bulkUnblockPackagesMock,
  invalidatePackageMock,
} = vi.hoisted(() => ({
  blockPackageMock: vi.fn(),
  unblockPackageMock: vi.fn(),
  bulkBlockPackagesMock: vi.fn(),
  bulkUnblockPackagesMock: vi.fn(),
  invalidatePackageMock: vi.fn(),
}));

vi.mock("@/client/sdk.gen", () => ({
  blockPackage: blockPackageMock,
  unblockPackage: unblockPackageMock,
  bulkBlockPackages: bulkBlockPackagesMock,
  bulkUnblockPackages: bulkUnblockPackagesMock,
  invalidatePackage: invalidatePackageMock,
}));

import PackageVersionsTable from "./PackageVersionsTable.vue";

function version(over: Partial<PackageVersionDetail> = {}): PackageVersionDetail {
  return {
    id: "v1",
    version: "1.0.0",
    artifact: "pkg-1.0.0.tgz",
    cached: false,
    cached_at: null,
    access_count: 0,
    last_accessed: null,
    last_accessed_by: null,
    socket_badge_url: null,
    status: { status: "available" },
    storage_backend: null,
    storage_key: "npm/pkg/1.0.0",
    license: null,
    vulnerabilities: [],
    ...over,
  } as PackageVersionDetail;
}

async function mountComp(versions: PackageVersionDetail[]) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/", component: { template: "<div />" } },
      { path: "/packages/:registry/:name", component: { template: "<div />" } },
    ],
  });
  await router.push("/");
  await router.isReady();
  const wrapper = mount(PackageVersionsTable, {
    props: { registry: "npm", name: "pkg", versions },
    global: { plugins: [router] },
  });
  return { wrapper, router };
}

describe("PackageVersionsTable", () => {
  beforeEach(() => {
    blockPackageMock.mockReset().mockResolvedValue({});
    unblockPackageMock.mockReset().mockResolvedValue({});
    bulkBlockPackagesMock
      .mockReset()
      .mockResolvedValue({ data: { succeeded_count: 1, failed_count: 0 } });
    bulkUnblockPackagesMock
      .mockReset()
      .mockResolvedValue({ data: { succeeded_count: 1, failed_count: 0 } });
    invalidatePackageMock.mockReset().mockResolvedValue({});
    vi.spyOn(globalThis, "confirm").mockReturnValue(true);
    vi.spyOn(globalThis, "prompt").mockReturnValue("bad license");
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("shows the empty state when there are no versions", async () => {
    const { wrapper } = await mountComp([]);
    expect(wrapper.text()).toContain("No versions tracked yet.");
  });

  it("renders a row per version with a pre-release badge", async () => {
    const { wrapper } = await mountComp([version(), version({ id: "v2", version: "2.0.0-rc.1" })]);
    const rows = wrapper.findAll("tbody tr");
    expect(rows).toHaveLength(2);
    expect(rows[1].text()).toContain("pre-release");
    expect(rows[0].text()).not.toContain("pre-release");
  });

  it("shows vulnerability badges by severity", async () => {
    const { wrapper } = await mountComp([
      version({
        vulnerabilities: [
          {
            osv_id: "OSV-1",
            summary: "bad",
            severity: "critical",
            fixed_version: "1.0.1",
            purl: "pkg:npm/test@1.0.0",
          },
          {
            osv_id: "OSV-2",
            summary: "meh",
            severity: "low",
            fixed_version: null,
            purl: "pkg:npm/test@1.0.0",
          },
        ],
      }),
    ]);
    expect(wrapper.text()).toContain("critical");
    expect(wrapper.text()).toContain("low");
  });

  it("navigates to the detail page on View", async () => {
    const { wrapper, router } = await mountComp([version()]);
    const push = vi.spyOn(router, "push");
    const viewBtn = wrapper.findAll("button").find((b) => b.text() === "View")!;
    await viewBtn.trigger("click");
    /* The package is named by the path now; version and artifact stay as query
       because they select within a package rather than name a different one. */
    expect(push).toHaveBeenCalledWith({
      path: "/packages/npm/pkg",
      query: { version: "1.0.0", artifact: "pkg-1.0.0.tgz" },
    });
  });

  it("blocks a version via the prompt reason and emits reload", async () => {
    const { wrapper } = await mountComp([version()]);
    const blockBtn = wrapper.findAll("button").find((b) => b.text() === "Block")!;
    await blockBtn.trigger("click");
    await flushPromises();
    expect(blockPackageMock).toHaveBeenCalledWith({
      body: {
        registry: "npm",
        name: "pkg",
        version: "1.0.0",
        artifact: "pkg-1.0.0.tgz",
        reason: "bad license",
      },
    });
    expect(wrapper.emitted("reload")).toHaveLength(1);
  });

  it("does not block when the prompt is cancelled", async () => {
    vi.spyOn(globalThis, "prompt").mockReturnValue(null);
    const { wrapper } = await mountComp([version()]);
    const blockBtn = wrapper.findAll("button").find((b) => b.text() === "Block")!;
    await blockBtn.trigger("click");
    expect(blockPackageMock).not.toHaveBeenCalled();
  });

  it("unblocks a blocked version", async () => {
    const { wrapper } = await mountComp([
      version({ status: { status: "blocked", blocked_at: "t", blocked_by: "admin", reason: "r" } }),
    ]);
    expect(wrapper.text()).toContain("Blocked");
    expect(wrapper.text()).toContain("r");
    const unblockBtn = wrapper.findAll("button").find((b) => b.text() === "Unblock")!;
    await unblockBtn.trigger("click");
    await flushPromises();
    expect(unblockPackageMock).toHaveBeenCalled();
    expect(wrapper.emitted("reload")).toHaveLength(1);
  });

  it("purges the cache for a cached version after confirm", async () => {
    const { wrapper } = await mountComp([version({ cached: true, cached_at: "2026-01-01" })]);
    expect(wrapper.text()).toContain("Cached");
    const purgeBtn = wrapper.findAll("button").find((b) => b.text() === "Purge cache")!;
    await purgeBtn.trigger("click");
    await flushPromises();
    expect(invalidatePackageMock).toHaveBeenCalled();
    expect(wrapper.emitted("reload")).toHaveLength(1);
  });

  it("skips purge when confirm is declined", async () => {
    vi.spyOn(globalThis, "confirm").mockReturnValue(false);
    const { wrapper } = await mountComp([version({ cached: true })]);
    const purgeBtn = wrapper.findAll("button").find((b) => b.text() === "Purge cache")!;
    await purgeBtn.trigger("click");
    expect(invalidatePackageMock).not.toHaveBeenCalled();
  });

  it("selects all versions and bulk-blocks them", async () => {
    const { wrapper } = await mountComp([version(), version({ id: "v2", version: "2.0.0" })]);
    const selectAll = wrapper.find('input[aria-label="Select all versions"]');
    await selectAll.setValue(true);
    /* Real plural forms now, rather than "(s)" — vue-i18n picks the form. */
    expect(wrapper.text()).toContain("2 versions selected");

    const bulkBlockBtn = wrapper.findAll("button").find((b) => b.text() === "Block selected")!;
    await bulkBlockBtn.trigger("click");
    await flushPromises();

    expect(bulkBlockPackagesMock).toHaveBeenCalledWith({
      body: {
        items: [
          {
            registry: "npm",
            name: "pkg",
            version: "1.0.0",
            artifact: "pkg-1.0.0.tgz",
            reason: "bad license",
          },
          {
            registry: "npm",
            name: "pkg",
            version: "2.0.0",
            artifact: "pkg-1.0.0.tgz",
            reason: "bad license",
          },
        ],
      },
    });
    // The catalogue owns the whole sentence and pluralises the noun, so the
    // singular case reads "1 version" rather than the assembled "1 version(s)".
    expect((wrapper.vm as unknown as { bulkMsg: string }).bulkMsg).toBe("Blocked 1 version.");
    expect(wrapper.emitted("reload")).toHaveLength(1);
  });

  it("bulk-unblocks selected versions after confirm", async () => {
    const { wrapper } = await mountComp([version()]);
    await wrapper.find(`input[aria-label="Select version 1.0.0"]`).setValue(true);
    const bulkUnblockBtn = wrapper.findAll("button").find((b) => b.text() === "Unblock selected")!;
    await bulkUnblockBtn.trigger("click");
    await flushPromises();
    expect(bulkUnblockPackagesMock).toHaveBeenCalled();
    expect((wrapper.vm as unknown as { bulkMsg: string }).bulkMsg).toBe("Unblocked 1 version.");
  });

  it("clears the selection via Clear", async () => {
    const { wrapper } = await mountComp([version()]);
    await wrapper.find(`input[aria-label="Select version 1.0.0"]`).setValue(true);
    const clearBtn = wrapper.findAll("button").find((b) => b.text() === "Clear")!;
    await clearBtn.trigger("click");
    expect(wrapper.text()).not.toContain("selected");
  });

  it("shows the declared licence", async () => {
    const { wrapper } = await mountComp([version({ license: "MIT OR Apache-2.0" })]);
    expect(wrapper.text()).toContain("MIT OR Apache-2.0");
  });

  /**
   * RFC 0004-bis §13.1: a null licence means the manifest was never read, not
   * that the package is unlicensed. Rendering nothing would make the two
   * indistinguishable — the §2.4 defect, a blank that reads as a fact — so the
   * absence is stated and the title says why.
   */
  it("states that an absent licence is unknown, not absent", async () => {
    const { wrapper } = await mountComp([version({ license: null })]);
    expect(wrapper.text()).toContain("licence unknown");
    const cell = wrapper.findAll("p").find((p) => p.text() === "licence unknown")!;
    expect(cell.attributes("title")).toContain("not the same as unlicensed");
  });
});
