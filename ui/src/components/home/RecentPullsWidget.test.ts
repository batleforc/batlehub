import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";

const { myDownloadsMock } = vi.hoisted(() => ({ myDownloadsMock: vi.fn() }));
vi.mock("@/client/sdk.gen", () => ({ myDownloads: myDownloadsMock }));

import RecentPullsWidget from "./RecentPullsWidget.vue";

const pull = (over: Record<string, unknown> = {}) => ({
  registry: "npm",
  name: "left-pad",
  version: "1.0.0",
  downloaded_at: new Date(Date.now() - 5 * 60_000).toISOString(),
  ...over,
});

async function mountWidget() {
  const wrapper = mount(RecentPullsWidget, {
    global: { stubs: { RouterLink: { template: "<a><slot /></a>" } } },
  });
  await flushPromises();
  return wrapper;
}

describe("RecentPullsWidget", () => {
  beforeEach(() => {
    myDownloadsMock.mockReset().mockResolvedValue({
      data: [pull(), pull({ registry: "cargo", name: "serde", version: "1.0.203" })],
    });
  });

  it("lists what the caller pulled, with registry and coordinate", async () => {
    const wrapper = await mountWidget();
    expect(wrapper.findAll("li")).toHaveLength(2);
    expect(wrapper.text()).toContain("left-pad");
    expect(wrapper.text()).toContain("1.0.0");
    expect(wrapper.text()).toContain("serde");
    expect(wrapper.text()).toContain("cargo");
  });

  /**
   * Bounded on purpose: this is the glance, not the audit trail. The full
   * history is the endpoint's own `limit`, and the admin surface is
   * `/admin/observability/audit-log`.
   */
  it("asks the server for a bounded window rather than everything", async () => {
    await mountWidget();
    expect(myDownloadsMock).toHaveBeenCalledWith(
      expect.objectContaining({ query: expect.objectContaining({ limit: expect.any(Number) }) }),
    );
  });

  it("shows when, relatively, and keeps the exact time reachable", async () => {
    const wrapper = await mountWidget();
    expect(wrapper.text()).toMatch(/ago|just now/i);
    const stamp = wrapper.findAll("span").find((s) => s.attributes("title"));
    expect(stamp?.attributes("title")).toMatch(/^\d{4}-\d{2}-\d{2}T/);
  });

  /**
   * "Nothing yet" is informative here: the most common reason a registry proxy
   * looks idle is that the developer's tooling was never actually pointed at
   * it, and this is the one surface that would tell them.
   */
  it("gives a real answer when nothing has been pulled", async () => {
    myDownloadsMock.mockResolvedValue({ data: [] });
    const wrapper = await mountWidget();
    expect(wrapper.find('[data-testid="recent-pulls-empty"]').exists()).toBe(true);
    expect(wrapper.text()).toMatch(/token/i);
  });

  it("surfaces an error rather than an empty list", async () => {
    myDownloadsMock.mockResolvedValue({ error: { message: "boom" } });
    const wrapper = await mountWidget();
    expect(wrapper.text()).toContain("boom");
    expect(wrapper.find('[data-testid="recent-pulls-empty"]').exists()).toBe(false);
  });

  /**
   * The same coordinate fetched as two different files is two pulls, and the
   * key has to keep them apart or Vue reuses one node for both.
   */
  it("keeps two artifacts of one coordinate as two rows", async () => {
    myDownloadsMock.mockResolvedValue({
      data: [pull({ artifact: "tarball" }), pull({ artifact: "metadata" })],
    });
    const wrapper = await mountWidget();
    expect(wrapper.findAll("li")).toHaveLength(2);
  });
});
