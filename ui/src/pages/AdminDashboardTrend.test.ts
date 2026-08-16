import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";

const { adminStatsMock, adminStatsHistoryMock, registryHealthMock } = vi.hoisted(() => ({
  adminStatsMock: vi.fn(),
  adminStatsHistoryMock: vi.fn(),
  registryHealthMock: vi.fn(),
}));

vi.mock("@/client/sdk.gen", () => ({
  adminStats: adminStatsMock,
  adminStatsHistory: adminStatsHistoryMock,
  registryHealth: registryHealthMock,
}));

import AdminDashboard from "./AdminDashboard.vue";

const stats = {
  since_startup: "2026-08-12T10:00:00Z",
  aggregate: { artifact_hits: 90, artifact_misses: 10, hit_rate: 0.9, cached_bytes: 1024 },
  per_registry: [],
};

const trend = (over: Record<string, unknown> = {}) => ({
  window_days: 30,
  from: "2026-07-13T00:00:00Z",
  to: "2026-08-12T00:00:00Z",
  trend: { hits: 90, misses: 10, hit_rate: 0.9, previous_hit_rate: 0.8, delta: 0.1, ...over },
  per_registry: [],
});

async function mountDashboard() {
  const wrapper = mount(AdminDashboard, {
    global: { stubs: { RouterLink: { template: "<a><slot /></a>" } } },
  });
  await flushPromises();
  return wrapper;
}

/**
 * The trend is the one thing `/api/v1/admin/stats` structurally cannot say:
 * its counters are `since_startup` and a deploy sets them to zero
 * (RFC 0004 §2.3). These assertions are about the *sentence*, because §4.3
 * asks for something a reader can act on rather than a line that only says
 * "something changed".
 */
describe("AdminDashboard verdict", () => {
  beforeEach(() => {
    adminStatsMock.mockReset().mockResolvedValue({ data: stats });
    registryHealthMock.mockReset();
    adminStatsHistoryMock.mockReset().mockResolvedValue({ data: trend() });
  });

  /**
   * `useApi` nulls `data` on error, so an unreadable health probe made the page
   * conclude the instance was empty and tell a paged operator to add a
   * `[[registries]]` block — on an instance serving real traffic.
   */
  it("does not call the instance empty when the health probe fails", async () => {
    registryHealthMock.mockResolvedValue({ error: { message: "boom" } });
    const wrapper = await mountDashboard();
    expect(wrapper.text()).not.toMatch(/No registries configured/i);
    expect(wrapper.text()).toMatch(/could not be read/i);
  });

  /**
   * `error_type` is `"denied"` (RBAC refusing a developer — the policy engine
   * working) or `"error"` (something actually broken). Alarming on the first
   * pages an operator because the product did its job.
   */
  it("does not raise an alarm for policy denials", async () => {
    registryHealthMock.mockResolvedValue({
      data: [
        { registry: "npm", recent_errors: [{ error_type: "denied", reason: "rbac" }] },
        { registry: "cargo", recent_errors: [] },
      ],
    });
    const wrapper = await mountDashboard();
    expect(wrapper.text()).toMatch(/are answering/i);
    expect(wrapper.text()).not.toMatch(/reported errors/i);
  });

  it("raises an alarm for a real fault, and names the registry", async () => {
    registryHealthMock.mockResolvedValue({
      data: [
        { registry: "npm", recent_errors: [{ error_type: "error", reason: "upstream timeout" }] },
        { registry: "cargo", recent_errors: [] },
      ],
    });
    const wrapper = await mountDashboard();
    expect(wrapper.text()).toMatch(/reported errors/i);
    expect(wrapper.text()).toContain("npm");
  });

  /**
   * The verdict was three hardcoded English sentences while `dashboard.*`
   * held translated equivalents that nothing referenced — so a French
   * operator's 3am alarm arrived in English.
   */
  it("resolves the verdict through the catalogue", async () => {
    registryHealthMock.mockResolvedValue({ data: [{ registry: "npm", recent_errors: [] }] });
    const wrapper = await mountDashboard();
    expect(wrapper.text()).not.toContain("dashboard.allAnswering");
    expect(wrapper.text()).toMatch(/answering/i);
  });
});

describe("AdminDashboard trend", () => {
  beforeEach(() => {
    adminStatsMock.mockReset().mockResolvedValue({ data: stats });
    registryHealthMock
      .mockReset()
      .mockResolvedValue({ data: [{ registry: "npm", recent_errors: [] }] });
    adminStatsHistoryMock.mockReset().mockResolvedValue({ data: trend() });
  });

  it("says how much better, in points, against the previous window", async () => {
    const wrapper = await mountDashboard();
    const text = wrapper.get('[data-testid="dashboard-trend"]').text();
    expect(text).toContain("30");
    expect(text).toContain("90.0%");
    expect(text).toContain("10.0 points");
    expect(text).toMatch(/better/i);
  });

  it("says worse, and reads as a warning rather than as neutral prose", async () => {
    adminStatsHistoryMock.mockResolvedValue({
      data: trend({ hit_rate: 0.7, previous_hit_rate: 0.9, delta: -0.2 }),
    });
    const wrapper = await mountDashboard();
    const el = wrapper.get('[data-testid="dashboard-trend"]');
    expect(el.text()).toMatch(/worse/i);
    expect(el.text()).toContain("20.0 points");
    expect(el.classes()).toContain("text-copper");
  });

  /**
   * A tenth of a point either way is a rounding artefact, and calling it a
   * direction has an operator chasing noise.
   */
  it("calls a negligible change unchanged", async () => {
    adminStatsHistoryMock.mockResolvedValue({
      data: trend({ hit_rate: 0.9, previous_hit_rate: 0.9004, delta: -0.0004 }),
    });
    const wrapper = await mountDashboard();
    expect(wrapper.get('[data-testid="dashboard-trend"]').text()).toMatch(/unchanged/i);
  });

  /**
   * The careful case: no previous window is "not enough history yet", which is
   * a different statement from "no change" — and the one a fresh instance is
   * actually in.
   */
  it("does not claim stability when there is no baseline", async () => {
    adminStatsHistoryMock.mockResolvedValue({
      data: trend({ previous_hit_rate: null, delta: null }),
    });
    const wrapper = await mountDashboard();
    const text = wrapper.get('[data-testid="dashboard-trend"]').text();
    expect(text).toMatch(/not enough history/i);
    expect(text).not.toMatch(/unchanged/i);
  });

  it("renders no trend at all when the window saw no traffic", async () => {
    adminStatsHistoryMock.mockResolvedValue({
      data: trend({ hits: 0, misses: 0, hit_rate: null, previous_hit_rate: null, delta: null }),
    });
    const wrapper = await mountDashboard();
    expect(wrapper.find('[data-testid="dashboard-trend"]').exists()).toBe(false);
  });

  /**
   * `history_enabled = false` restores the previous dashboard. The page must
   * still render its verdict and counters, not fail because a series it asked
   * for was not there.
   */
  it("keeps working when history is unavailable", async () => {
    adminStatsHistoryMock.mockResolvedValue({ error: { message: "no history" } });
    const wrapper = await mountDashboard();
    expect(wrapper.find('[data-testid="dashboard-trend"]').exists()).toBe(false);
    expect(wrapper.text()).toContain("90.0%");
  });
});
