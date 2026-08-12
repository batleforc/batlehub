import { mount } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";

import AccessCheck from "./AccessCheck.vue";

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

describe("AccessCheck prefill", () => {
  it("fills the coordinate from the query it was linked with", () => {
    mockRoute = { query: { registry: "npm1", name: "left-pad", version: "1.3.0" } };
    const wrapper = mountWith({});

    expect((wrapper.find("#registry").element as HTMLInputElement).value).toBe("npm1");
    expect((wrapper.find("#name").element as HTMLInputElement).value).toBe("left-pad");
    expect((wrapper.find("#version").element as HTMLInputElement).value).toBe("1.3.0");
  });

  /** Opened directly from the nav, it still offers a usable starting point. */
  it("falls back to its defaults when opened without a query", () => {
    mockRoute = { query: {} };
    const wrapper = mountWith({});

    expect((wrapper.find("#registry").element as HTMLInputElement).value).toBe("github");
    expect((wrapper.find("#name").element as HTMLInputElement).value).toBe("");
  });

  /** A repeated query param arrives as an array; it must not render as one. */
  it("ignores a non-string query value rather than rendering it", () => {
    mockRoute = { query: { registry: ["npm1", "npm2"] } };
    const wrapper = mountWith({});

    expect((wrapper.find("#registry").element as HTMLInputElement).value).toBe("github");
  });
});
