import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import Announcer from "./Announcer.vue";

describe("Announcer", () => {
  it("is a polite status region by default", () => {
    const w = mount(Announcer, { props: { message: "20 packages loaded." } });
    expect(w.attributes("role")).toBe("status");
    expect(w.attributes("aria-live")).toBe("polite");
    expect(w.text()).toBe("20 packages loaded.");
  });

  /** Assertive interrupts whatever is being read; reserved for errors. */
  it("escalates to alert when assertive", () => {
    const w = mount(Announcer, { props: { message: "Upstream unreachable.", assertive: true } });
    expect(w.attributes("role")).toBe("alert");
    expect(w.attributes("aria-live")).toBe("assertive");
  });

  /** Atomic, or a partial update reads as a fragment of a sentence. */
  it("announces the whole message rather than the diff", () => {
    expect(mount(Announcer).attributes("aria-atomic")).toBe("true");
  });

  it("is visually hidden — it exists only for assistive tech", () => {
    expect(mount(Announcer).classes()).toContain("sr-only");
  });
});
