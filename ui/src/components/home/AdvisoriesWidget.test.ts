import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";

const { myAdvisoriesMock } = vi.hoisted(() => ({ myAdvisoriesMock: vi.fn() }));
vi.mock("@/client/sdk.gen", () => ({ myAdvisories: myAdvisoriesMock }));

import AdvisoriesWidget from "./AdvisoriesWidget.vue";

const advisory = (over: Record<string, unknown> = {}) => ({
  registry: "npm",
  name: "left-pad",
  version: "1.0.0",
  relation: "pulled",
  highest_severity: "high",
  findings: [
    {
      osv_id: "GHSA-xxxx",
      severity: "high",
      summary: "Prototype pollution",
      fixed_version: "1.0.1",
    },
  ],
  ...over,
});

const response = (over: Record<string, unknown> = {}) => ({
  advisories: [advisory()],
  window_days: 7,
  scanning_available: true,
  ...over,
});

async function mountWidget() {
  const wrapper = mount(AdvisoriesWidget, {
    global: { stubs: { RouterLink: { template: "<a><slot /></a>" } } },
  });
  await flushPromises();
  return wrapper;
}

describe("AdvisoriesWidget", () => {
  beforeEach(() => {
    myAdvisoriesMock.mockReset().mockResolvedValue({ data: response() });
  });

  // ── the four states RFC 0004 §10 asks for ────────────────────────────────

  it("lists a coordinate with its findings", async () => {
    const wrapper = await mountWidget();
    expect(wrapper.text()).toContain("left-pad");
    expect(wrapper.text()).toContain("1.0.0");
    expect(wrapper.text()).toContain("GHSA-xxxx");
    expect(wrapper.text()).toContain("Prototype pollution");
    expect(wrapper.text()).toContain("1.0.1");
  });

  it("surfaces an error rather than an empty list", async () => {
    myAdvisoriesMock.mockResolvedValue({ error: { message: "boom" } });
    const wrapper = await mountWidget();
    expect(wrapper.text()).toContain("boom");
    expect(wrapper.find('[data-testid="advisories-clear"]').exists()).toBe(false);
  });

  /**
   * RFC 0004 §4.2 wants a real answer, not a blank — and the answer is only
   * meaningful against the window it covers.
   */
  it("gives a real answer when nothing is affected, naming the window", async () => {
    myAdvisoriesMock.mockResolvedValue({ data: response({ advisories: [] }) });
    const wrapper = await mountWidget();
    const empty = wrapper.get('[data-testid="advisories-clear"]');
    expect(empty.text()).toContain("7");
    expect(wrapper.find('[data-testid="advisories-unknown"]').exists()).toBe(false);
  });

  /**
   * The one thing this widget must never do. With no SBOM re-scan configured
   * the instance records no findings at all, so an empty list means "we do not
   * know" — telling a reader they are clear would be a false assurance about
   * their own supply chain.
   */
  it("never reports 'clear' when nothing is scanning", async () => {
    myAdvisoriesMock.mockResolvedValue({
      data: response({ advisories: [], scanning_available: false }),
    });
    const wrapper = await mountWidget();

    expect(wrapper.find('[data-testid="advisories-unknown"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="advisories-clear"]').exists()).toBe(false);
    // …and the copy says so in words, not only by which block rendered.
    expect(wrapper.text().toLowerCase()).toContain("unknown");
  });

  // ── the two relationships ────────────────────────────────────────────────

  /**
   * RFC 0004 R7: you are *exposed to* what you pulled and can *fix* what you
   * own. Merging them loses the only thing that tells a reader which of the
   * two they can act on.
   */
  it("labels the two relationships differently", async () => {
    myAdvisoriesMock.mockResolvedValue({
      data: response({
        advisories: [
          advisory({ relation: "pulled", name: "dep" }),
          advisory({ relation: "owned", name: "mine" }),
        ],
      }),
    });
    const wrapper = await mountWidget();
    const rows = wrapper.findAll("ul > li");
    const pulled = rows.find((r) => r.text().includes("dep"))!;
    const owned = rows.find((r) => r.text().includes("mine"))!;
    expect(pulled.text()).not.toBe(owned.text());
    expect(wrapper.text()).toContain("Pulled");
    expect(wrapper.text()).toContain("Owned");
  });

  /**
   * R15: a version you pulled and a different version you own are two
   * coordinates, so both get a row and both name their version.
   */
  it("keeps two versions of one package as two rows", async () => {
    myAdvisoriesMock.mockResolvedValue({
      data: response({
        advisories: [
          advisory({ version: "1.0.0", relation: "pulled" }),
          advisory({ version: "2.0.0", relation: "owned" }),
        ],
      }),
    });
    const wrapper = await mountWidget();
    expect(wrapper.text()).toContain("1.0.0");
    expect(wrapper.text()).toContain("2.0.0");
  });

  it("renders a finding with no known fix", async () => {
    myAdvisoriesMock.mockResolvedValue({
      data: response({
        advisories: [
          advisory({
            findings: [
              { osv_id: "GHSA-yyyy", severity: "low", summary: "Something", fixed_version: null },
            ],
          }),
        ],
      }),
    });
    const wrapper = await mountWidget();
    expect(wrapper.text()).toContain("GHSA-yyyy");
    expect(wrapper.text()).not.toMatch(/fixed in\s*$/i);
  });

  it("translates the severity rather than printing the wire value", async () => {
    myAdvisoriesMock.mockResolvedValue({
      data: response({ advisories: [advisory({ highest_severity: "critical" })] }),
    });
    const wrapper = await mountWidget();
    expect(wrapper.text()).toContain("Critical");
  });
});
