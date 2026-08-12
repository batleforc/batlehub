import { mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it } from "vitest";
import { nextTick } from "vue";

import ThemeToggle from "./ThemeToggle.vue";

/**
 * The theme is a three-state *preference* (RFC 0003 R12), not a two-way switch.
 *
 * The distinction these tests protect: "follow the system" has to be storable.
 * A toggle that only remembers light-or-dark silently freezes the user's choice
 * at whatever the OS happened to be on the day they first clicked it, and no
 * later OS change reaches them.
 */
describe("ThemeToggle", () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute("data-theme");
  });

  it("starts on the system preference rather than a hardcoded theme", () => {
    const wrapper = mount(ThemeToggle);
    expect(wrapper.attributes("aria-label")).toBe("Theme: follow system");
  });

  it("cycles system → light → dark → system", async () => {
    const wrapper = mount(ThemeToggle);
    const seen: (string | undefined)[] = [wrapper.attributes("aria-label")];

    for (let i = 0; i < 3; i++) {
      await wrapper.trigger("click");
      await nextTick();
      seen.push(wrapper.attributes("aria-label"));
    }

    expect(seen).toEqual([
      "Theme: follow system",
      "Theme: light",
      "Theme: dark",
      "Theme: follow system",
    ]);
  });

  /** The token layer keys off data-theme; a class would style nothing. */
  it("writes the resolved rendition to data-theme on <html>", async () => {
    const wrapper = mount(ThemeToggle);

    await wrapper.trigger("click"); // light
    await nextTick();
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");

    await wrapper.trigger("click"); // dark
    await nextTick();
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
  });

  /** Persisted under the same key the design proof and DESIGN.md name. */
  it("stores the preference, not the resolved value", async () => {
    const wrapper = mount(ThemeToggle);

    await wrapper.trigger("click");
    await nextTick();
    expect(localStorage.getItem("batlehub.theme")).toBe("light");

    await wrapper.trigger("click");
    await wrapper.trigger("click"); // back to auto
    await nextTick();
    expect(localStorage.getItem("batlehub.theme")).toBe("auto");
  });

  it("always exposes an accessible name", async () => {
    const wrapper = mount(ThemeToggle);
    for (let i = 0; i < 3; i++) {
      expect(wrapper.attributes("aria-label")).toBeTruthy();
      await wrapper.trigger("click");
      await nextTick();
    }
  });
});
