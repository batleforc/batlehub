import { mount, flushPromises } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";

const { checkAccessMock } = vi.hoisted(() => ({ checkAccessMock: vi.fn() }));
vi.mock("@/client/sdk.gen", () => ({
  checkAccess: checkAccessMock,
  listRegistries: vi.fn().mockResolvedValue({ data: [] }),
  explorePackages: vi.fn().mockResolvedValue({ data: { items: [] } }),
  listPackageVersions: vi.fn().mockResolvedValue({ data: { items: [] } }),
}));

import AccessCheck from "./AccessCheck.vue";
import { Badge } from "@/components/ui/badge";
import { Resolution } from "@/components/ui/resolution";

/**
 * The denial → diagnostics path (RFC 0003 §4.4).
 *
 * Nobody opens an access checker for fun; they open it because something was
 * refused. Arriving with the coordinate already filled in is the difference
 * between a tool that gets used and one that gets closed — retyping what you
 * just read on the previous screen is exactly the friction that makes a
 * diagnostic sit unused while people guess instead.
 */
const mountWith = (query: Record<string, string>) =>
  mount(AccessCheck, {
    global: {
      mocks: { $route: { query } },
      stubs: { RouterLink: true },
    },
  });

vi.mock("vue-router", () => ({
  useRoute: () => mockRoute,
  RouterLink: { template: "<a><slot/></a>" },
}));

let mockRoute: { query: Record<string, unknown> } = { query: {} };

/**
 * The registry field is a `Select` since RFC 0004-bis §6.2 — the set is closed,
 * small and already fetched, and four pages were each guessing a different
 * naming convention in a placeholder. Its value lives in the component's state
 * rather than in an `<input>`, so the prefill is asserted there; the two
 * free-text coordinates are still read off the DOM.
 */
const registryValue = (wrapper: ReturnType<typeof mountWith>) =>
  (wrapper.vm as unknown as { registry: string }).registry;

describe("AccessCheck prefill", () => {
  it("fills the coordinate from the query it was linked with", () => {
    mockRoute = { query: { registry: "npm1", name: "left-pad", version: "1.3.0" } };
    const wrapper = mountWith({});

    expect(registryValue(wrapper)).toBe("npm1");
    expect((wrapper.find("#name").element as HTMLInputElement).value).toBe("left-pad");
    expect((wrapper.find("#version").element as HTMLInputElement).value).toBe("1.3.0");
  });

  /** Opened directly from the nav, it still offers a usable starting point. */
  it("falls back to its defaults when opened without a query", () => {
    mockRoute = { query: {} };
    const wrapper = mountWith({});

    expect(registryValue(wrapper)).toBe("github");
    expect((wrapper.find("#name").element as HTMLInputElement).value).toBe("");
  });

  /** A repeated query param arrives as an array; it must not render as one. */
  it("ignores a non-string query value rather than rendering it", () => {
    mockRoute = { query: { registry: ["npm1", "npm2"] } };
    const wrapper = mountWith({});

    expect(registryValue(wrapper)).toBe("github");
  });
});

/**
 * The answer, in three channels.
 *
 * `--destructive` resolves to `--accent` (`assets/index.css`), so
 * `variant="default"` and `variant="destructive"` paint the same crimson. This
 * page's only job is to answer "was I allowed?", and it rendered both answers
 * identically — one channel, hue, saying nothing.
 *
 * DESIGN.md: "Never colour alone — pattern, word and hue all carry it."
 */
describe("AccessCheck answer", () => {
  const answerWith = async (canAccess: boolean) => {
    mockRoute = { query: {} };
    checkAccessMock.mockResolvedValue({
      data: {
        can_access: canAccess,
        reason: canAccess ? null : "blocked by rule",
        proxy_url: null,
      },
    });
    const wrapper = mountWith({});
    // Not `find("button")`: the registry Select and both Comboboxes render
    // triggers of their own, and the first one in the DOM is a radix element.
    const submit = wrapper.findAll("button").find((b) => /check access/i.test(b.text()))!;
    await submit.trigger("click");
    await flushPromises();
    return wrapper;
  };

  it("does not paint an allowed answer in the refusal hue", async () => {
    const wrapper = await answerWith(true);
    const badge = wrapper.findComponent(Badge);
    expect(badge.props("variant")).toBe("known");
  });

  it("paints a refusal in the refusal hue", async () => {
    const wrapper = await answerWith(false);
    expect(wrapper.findComponent(Badge).props("variant")).toBe("destructive");
  });

  /**
   * The channel that survives a monochrome display and a colour-blind reader.
   * `Resolution`'s matrix is `aria-hidden` and it requires its own `label`, so
   * this costs nothing in the accessible name — the word is still the word.
   */
  it("carries the answer in the pattern as well as the hue", async () => {
    const allowed = await answerWith(true);
    expect(allowed.findComponent(Resolution).props("state")).toBe("cached");
    expect(allowed.find("[data-testid=resolution-matrix]").attributes("aria-hidden")).toBe("true");

    const denied = await answerWith(false);
    expect(denied.findComponent(Resolution).props("state")).toBe("blocked");
  });

  it("still says the answer in words", async () => {
    expect((await answerWith(true)).text()).toContain("Allowed");
    expect((await answerWith(false)).text()).toContain("Denied");
  });
});
