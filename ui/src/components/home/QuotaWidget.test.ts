import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";

const { myQuotaMock } = vi.hoisted(() => ({ myQuotaMock: vi.fn() }));
vi.mock("@/client/sdk.gen", () => ({ myQuota: myQuotaMock }));

import QuotaWidget from "./QuotaWidget.vue";

const row = (over: Record<string, unknown> = {}) => ({
  registry: "npm",
  bytes_used: 100,
  bytes_limit: 1000,
  packages_used: 1,
  packages_limit: 10,
  warn_threshold_pct: 80,
  state: "ok",
  bytes_state: "ok",
  packages_state: "ok",
  ...over,
});

async function mountWidget() {
  const wrapper = mount(QuotaWidget);
  await flushPromises();
  return wrapper;
}

describe("QuotaWidget", () => {
  beforeEach(() => {
    myQuotaMock.mockReset().mockResolvedValue({ data: [row()] });
  });

  // ── the four states RFC 0004 §10 asks for ────────────────────────────────

  it("renders a meter per limited dimension", async () => {
    const wrapper = await mountWidget();
    const meters = wrapper.findAll('[role="meter"]');
    expect(meters).toHaveLength(2); // storage and versions
    expect(meters[0].attributes("aria-valuenow")).toBe("100");
    expect(meters[0].attributes("aria-valuemax")).toBe("1000");
  });

  it("says nothing extra while usage is ordinary", async () => {
    const wrapper = await mountWidget();
    expect(wrapper.find('[data-testid="quota-caption"]').exists()).toBe(false);
    expect(wrapper.get('[role="meter"]').attributes("data-state")).toBe("ok");
  });

  /**
   * The threshold verdict is the server's — the component must not recompute
   * it, or it can disagree with the 429 the same config produces.
   */
  it("carries the warning state through with a word, not only a hue", async () => {
    myQuotaMock.mockResolvedValue({
      data: [row({ bytes_used: 850, state: "warning", bytes_state: "warning" })],
    });
    const wrapper = await mountWidget();
    expect(wrapper.get('[role="meter"]').attributes("data-state")).toBe("warning");
    const caption = wrapper.get('[data-testid="quota-caption"]');
    expect(caption.text()).toContain("80%");
    expect(caption.text().length).toBeGreaterThan(0);
  });

  it("says a publish may be refused once the limit is reached", async () => {
    myQuotaMock.mockResolvedValue({
      data: [row({ bytes_used: 1000, state: "at_limit", bytes_state: "at_limit" })],
    });
    const wrapper = await mountWidget();
    expect(wrapper.get('[role="meter"]').attributes("data-state")).toBe("at-limit");
    expect(wrapper.get('[data-testid="quota-caption"]').text()).toMatch(/refus/i);
  });

  it("surfaces an error rather than an empty meter", async () => {
    myQuotaMock.mockResolvedValue({ error: { message: "boom" } });
    const wrapper = await mountWidget();
    expect(wrapper.text()).toContain("boom");
    expect(wrapper.find('[role="meter"]').exists()).toBe(false);
  });

  // ── what it refuses to draw ──────────────────────────────────────────────

  /**
   * RFC 0004 §4.2: "If none has one, the widget does not render — an empty
   * meter is worse than no meter." A meter with no limit invents a bound the
   * operator never configured.
   */
  it("renders nothing at all when no registry has a quota", async () => {
    myQuotaMock.mockResolvedValue({ data: [] });
    const wrapper = await mountWidget();
    expect(wrapper.find("section").exists()).toBe(false);
    expect(wrapper.text()).toBe("");
  });

  it("omits the meter for a dimension with no limit", async () => {
    myQuotaMock.mockResolvedValue({ data: [row({ packages_limit: null, packages_state: null })] });
    const wrapper = await mountWidget();
    expect(wrapper.findAll('[role="meter"]')).toHaveLength(1);
  });

  /**
   * The case that caught the row-level colouring: versions past the mark while
   * storage is nowhere near it. Colouring both meters the same claims storage
   * is in trouble when it is not.
   */
  it("colours each dimension by its own state, not the row's", async () => {
    myQuotaMock.mockResolvedValue({
      data: [
        row({
          bytes_used: 680,
          bytes_limit: 1000,
          bytes_state: "ok",
          packages_used: 41,
          packages_limit: 50,
          packages_state: "warning",
          state: "warning",
        }),
      ],
    });
    const wrapper = await mountWidget();
    const meters = wrapper.findAll('[role="meter"]');
    expect(meters[0].attributes("data-state")).toBe("ok");
    expect(meters[1].attributes("data-state")).toBe("warning");

    // …and exactly one caption, under the dimension it is about.
    expect(wrapper.findAll('[data-testid="quota-caption"]')).toHaveLength(1);
  });

  it("renders one entry per quota-gated registry", async () => {
    myQuotaMock.mockResolvedValue({
      data: [row(), row({ registry: "cargo", bytes_used: 500 })],
    });
    const wrapper = await mountWidget();
    expect(wrapper.text()).toContain("npm");
    expect(wrapper.text()).toContain("cargo");
    expect(wrapper.findAll('[role="meter"]')).toHaveLength(4);
  });
});
