import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";

const mocks = vi.hoisted(() => ({
  listPackages: vi.fn(),
  listRegistries: vi.fn(),
  blockPackage: vi.fn(),
  unblockPackage: vi.fn(),
  bulkBlockPackages: vi.fn(),
  bulkUnblockPackages: vi.fn(),
}));
vi.mock("@/client/sdk.gen", () => mocks);

const { authFetchMock } = vi.hoisted(() => ({ authFetchMock: vi.fn() }));
vi.mock("@/composables/useAuthFetch", () => ({
  useAuthFetch: () => ({ authFetch: authFetchMock }),
}));

vi.mock("vue-router", () => ({
  useRouter: () => ({ push: vi.fn() }),
  RouterLink: { template: "<a :href='String(to)'><slot/></a>", props: ["to"] },
}));

import AdminPackages from "./AdminPackages.vue";

const pkg = (over: Record<string, unknown> = {}) => ({
  id: "1",
  package_id: { registry: "npm", name: "left-pad", version: "1.0.0", artifact: null },
  status: { status: "available" },
  last_accessed: "2026-08-12T10:00:00Z",
  last_accessed_by: "oidc:alice",
  access_count: 7,
  ...over,
});

const listing = (items: unknown[]) => ({
  data: { items, total: items.length, page: 0, per_page: 1000 },
});

async function mountPage() {
  const wrapper = mount(AdminPackages, {
    attachTo: document.body,
    global: { stubs: { SectionTabs: true } },
  });
  await flushPromises();
  return wrapper;
}

/** The page's question: "what is cached here, and what is blocked". */
describe("AdminPackages", () => {
  beforeEach(() => {
    mocks.listPackages.mockReset().mockResolvedValue(listing([pkg()]));
    mocks.listRegistries.mockReset().mockResolvedValue({ data: [{ name: "npm", type: "npm" }] });
    mocks.bulkBlockPackages
      .mockReset()
      .mockResolvedValue({ data: { succeeded_count: 1, failed_count: 0 } });
    mocks.bulkUnblockPackages
      .mockReset()
      .mockResolvedValue({ data: { succeeded_count: 1, failed_count: 0 } });
    authFetchMock.mockReset();
  });

  afterEach(() => {
    document.body.innerHTML = "";
  });

  /**
   * §6.1: six columns, not ten.
   *
   * Measured at 1440 the table wanted ~1650px intrinsic in a 1134px container,
   * so the row verbs sat off-screen at the console's own standard width.
   */
  it("fits its columns, with the links on the cells they are about", async () => {
    const wrapper = await mountPage();

    const headers = wrapper.findAll("thead th").map((h) => h.text());
    // The checkbox, six named columns, and Actions.
    expect(headers).toHaveLength(8);
    // Exactly one blank head — the checkbox. The unlabelled *nav* column that
    // held two link buttons is gone; its links moved onto the name and version
    // cells, where a reader is already pointing.
    expect(headers.filter((h) => h === "")).toHaveLength(1);

    const row = wrapper.find("tbody tr");
    const links = row.findAll("a").map((a) => a.attributes("href"));
    expect(links.some((h) => h?.includes("/packages/npm/left-pad"))).toBe(true);
  });

  /**
   * §4.3: select-all → bulk block states its count before acting.
   *
   * A confirmation that does not say how many things it is about is a
   * confirmation nobody can actually give.
   */
  it("states the count before a bulk block runs", async () => {
    mocks.listPackages.mockResolvedValue(
      listing([
        pkg({ id: "1" }),
        pkg({
          id: "2",
          package_id: { registry: "npm", name: "lodash", version: "2.0.0", artifact: null },
        }),
      ]),
    );
    const wrapper = await mountPage();

    await wrapper.find('thead input[type="checkbox"]').setValue(true);
    await flushPromises();

    const blockBtn = wrapper.findAll("button").find((b) => /block selected/i.test(b.text()))!;
    await blockBtn.trigger("click");
    await flushPromises();

    // The count is in the dialog, and the request has not been sent yet.
    expect(document.body.textContent).toContain("2");
    expect(mocks.bulkBlockPackages).not.toHaveBeenCalled();
  });

  it("surfaces a load error rather than an empty catalogue", async () => {
    mocks.listPackages.mockResolvedValue({ error: { message: "db unreachable" } });
    const wrapper = await mountPage();
    expect(wrapper.text()).toContain("db unreachable");
  });

  it("says the catalogue is empty rather than looking broken", async () => {
    mocks.listPackages.mockResolvedValue(listing([]));
    const wrapper = await mountPage();
    expect(wrapper.text()).toMatch(/no packages/i);
  });
});
