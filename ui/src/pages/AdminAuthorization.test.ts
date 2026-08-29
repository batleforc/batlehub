/**
 * RFC 0015 §4.8's page.
 *
 * The assertions here are about the three things the page exists to make
 * unmissable rather than about markup: a shadow that is currently serving what
 * the model refuses, a `deny` that is not what the request receives, and the
 * difference between "no shadow" and "a quiet shadow" — which look identical in
 * an empty list and mean opposite things.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";

const mocks = vi.hoisted(() => ({
  adminAuthzExplain: vi.fn(),
  authzShadow: vi.fn(),
  listExemptions: vi.fn(),
  auditLog: vi.fn(),
  listRegistries: vi.fn(),
  explorePackages: vi.fn(),
  explorePackageDetail: vi.fn(),
}));
vi.mock("@/client/sdk.gen", () => mocks);

let mockRoute: { query: Record<string, unknown> } = { query: {} };
vi.mock("vue-router", () => ({
  useRoute: () => mockRoute,
  RouterLink: { template: "<a><slot/></a>" },
}));

import AdminAuthorization from "./AdminAuthorization.vue";

const noShadow = {
  data: { by_node: [], recent: [], kept: 500, no_shadow_configured: true },
};

const shadowServing = {
  data: {
    by_node: [
      {
        node: "namespace:@acme",
        shadow_until: "2099-12-01",
        count: 42,
        actions: ["releases:read"],
        subjects: ["role:user"],
        last_seen: "2026-08-29T00:00:00Z",
      },
    ],
    recent: [],
    kept: 500,
    no_shadow_configured: false,
  },
};

function explainAnswer(over: Record<string, unknown> = {}) {
  return {
    data: {
      decision: "deny",
      reason: "no grant for 'releases:read' on registry 'npm1'",
      resolved: [],
      tiers_walked: ["registry:npm1"],
      not_covered: ["per-package visibility (public/internal/team)"],
      attributes: {
        visibility: "public",
        prerelease_visibility: "public",
        immutable: "never",
        monotonic: false,
        versioning_dry_run: false,
        exempt_gates: [],
      },
      ...over,
    },
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mockRoute = { query: {} };
  mocks.authzShadow.mockResolvedValue(noShadow);
  mocks.listExemptions.mockResolvedValue({ data: [] });
  mocks.auditLog.mockResolvedValue({ data: [] });
  mocks.listRegistries.mockResolvedValue({ data: [] });
  mocks.explorePackages.mockResolvedValue({ data: { packages: [] } });
  mocks.explorePackageDetail.mockResolvedValue({ data: { versions: [] } });
});

async function mountPage() {
  const wrapper = mount(AdminAuthorization, { global: { stubs: { SectionTabs: true } } });
  await flushPromises();
  return wrapper;
}

describe("the authorization page", () => {
  /**
   * The three standing-risk panels load without being asked.
   *
   * An operator who has to press a button to discover a shadow is serving
   * everything has a page that does not do its job.
   */
  it("loads shadow, exemptions and denials on mount", async () => {
    await mountPage();
    expect(mocks.authzShadow).toHaveBeenCalled();
    expect(mocks.auditLog).toHaveBeenCalled();
  });

  /**
   * "No shadow" and "a quiet shadow" say different things.
   *
   * They look identical in an empty list and mean opposite things: the first
   * says enforcing is safe, the second says nothing was measured.
   */
  it("distinguishes no shadow from a quiet one", async () => {
    const none = await mountPage();
    expect(none.text()).toContain("No node is in shadow");

    mocks.authzShadow.mockResolvedValue({
      data: { by_node: [], recent: [], kept: 500, no_shadow_configured: false },
    });
    const quiet = await mountPage();
    expect(quiet.text()).toContain("has served nothing yet");
    expect(quiet.text()).not.toContain("No node is in shadow");
  });

  /** A shadow that is serving is shown with its node, expiry and count. */
  it("shows what a shadow is serving", async () => {
    mocks.authzShadow.mockResolvedValue(shadowServing);
    const w = await mountPage();
    const panel = w.get('[data-testid="panel-shadow"]').text();
    expect(panel).toContain("namespace:@acme");
    expect(panel).toContain("2099-12-01");
    expect(panel).toContain("42");
    expect(panel).toContain("were SERVED");
  });

  /**
   * **The assertion that matters most.** A `deny` under an active shadow is not
   * what the request receives.
   *
   * An operator reading a bare `DENY` would conclude the coordinate is closed
   * while every request to it succeeds — the exact misreading a shadow makes
   * possible, and §11.6's *"a diagnostic that can disagree with reality is worse
   * than none, because it is trusted."*
   */
  it("says when a denial is being shadowed", async () => {
    mocks.adminAuthzExplain.mockResolvedValue(
      explainAnswer({ shadowed_by: { node: "registry:npm1", until: "2099-12-01" } }),
    );
    const w = await mountPage();
    (w.vm as unknown as { registry: string }).registry = "npm1";
    await w.get("form").trigger("submit");
    await flushPromises();

    expect(w.find('[data-testid="explain-shadow-note"]').exists()).toBe(true);
    const note = w.get('[data-testid="explain-shadow-note"]').text();
    expect(note).toContain("registry:npm1");
    expect(note).toContain("2099-12-01");
  });

  /** …and without one the note is absent, so its presence always means something. */
  it("omits the shadow note when there is none", async () => {
    mocks.adminAuthzExplain.mockResolvedValue(explainAnswer());
    const w = await mountPage();
    (w.vm as unknown as { registry: string }).registry = "npm1";
    await w.get("form").trigger("submit");
    await flushPromises();

    expect(w.get('[data-testid="explain-result"]').text()).toContain("DENY");
    expect(w.find('[data-testid="explain-shadow-note"]').exists()).toBe(false);
  });

  /**
   * `granted_by` is a column, not a detail.
   *
   * §4.8: a resolved set without provenance tells an operator what they have;
   * naming the tier tells them which line to edit.
   */
  it("shows where each verb came from", async () => {
    mocks.adminAuthzExplain.mockResolvedValue(
      explainAnswer({
        decision: "allow",
        reason: null,
        resolved: [
          { action: "releases:read", granted_by: "namespace:@acme", subject: "group:*:qa" },
        ],
      }),
    );
    const w = await mountPage();
    (w.vm as unknown as { registry: string }).registry = "npm1";
    await w.get("form").trigger("submit");
    await flushPromises();

    const result = w.get('[data-testid="explain-result"]').text();
    expect(result).toContain("ALLOW");
    expect(result).toContain("namespace:@acme");
    expect(result).toContain("group:*:qa");
  });

  /** The `self_approved` filter §4.8 asks the exemptions panel for by name. */
  it("filters exemptions to the self-approved ones", async () => {
    mocks.listExemptions.mockResolvedValue({
      data: [
        {
          package: "lodash",
          version: "1.0.0",
          gate: "cve_gate",
          exempt_until: "2099-01-01T00:00:00Z",
          reason: "reviewed by security",
          granted_by: "sec",
          self_approved: false,
          expired: false,
        },
        {
          package: "left-pad",
          version: "2.0.0",
          gate: "license_gate",
          exempt_until: "2099-01-01T00:00:00Z",
          reason: "own build",
          granted_by: "bob",
          self_approved: true,
          expired: false,
        },
      ],
    });
    mockRoute = { query: { registry: "npm1" } };
    const w = await mountPage();

    let panel = w.get('[data-testid="panel-exemptions"]').text();
    expect(panel).toContain("lodash");
    expect(panel).toContain("left-pad");

    (w.vm as unknown as { selfApprovedOnly: boolean }).selfApprovedOnly = true;
    await flushPromises();
    panel = w.get('[data-testid="panel-exemptions"]').text();
    expect(panel).not.toContain("lodash");
    expect(panel).toContain("left-pad");
  });

  /** All five panels §4.8 names are on the page. */
  it("renders every panel", async () => {
    const w = await mountPage();
    for (const panel of ["shadow", "exemptions", "explain", "denials", "retention"]) {
      expect(w.find(`[data-testid="panel-${panel}"]`).exists()).toBe(true);
    }
  });
});
