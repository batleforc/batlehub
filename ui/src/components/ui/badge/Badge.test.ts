import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import { Badge } from ".";

describe("Badge", () => {
  it("renders slot content", () => {
    const wrapper = mount(Badge, { slots: { default: "Stable" } });
    expect(wrapper.text()).toBe("Stable");
    expect(wrapper.element.tagName).toBe("DIV");
  });

  it("applies the default variant classes", () => {
    const wrapper = mount(Badge, { slots: { default: "Stable" } });
    expect(wrapper.classes()).toContain("border-primary");
    expect(wrapper.classes()).toContain("text-primary");
  });

  /**
   * The accent variants carry no alpha fill. Accent text on a 10% tint of
   * itself measured 4.26:1 on paper — under AA, and unmeasurable by
   * construction, which is what the Undependable Fill Rule is about.
   */
  it("never puts accent text on a tint of itself", () => {
    for (const variant of ["default", "destructive", "copper"] as const) {
      const classes = mount(Badge, { props: { variant }, slots: { default: "x" } }).classes();
      expect(classes.filter((c) => /^bg-(primary|destructive|copper)\//.test(c))).toEqual([]);
    }
  });

  it("applies the requested variant classes", () => {
    const wrapper = mount(Badge, {
      props: { variant: "destructive" },
      slots: { default: "Yanked" },
    });
    expect(wrapper.classes()).toContain("border-destructive");
    expect(wrapper.classes()).toContain("text-destructive");
  });

  /**
   * The border is the state channel — the file says so, and at `/40` it did
   * not carry the state either: 1.76:1 in dark and 2.08:1 in light against the
   * 3:1 WCAG 1.4.11 asks of a boundary that is the only thing distinguishing a
   * state. Asserted as a property of the set, so a `/25` tomorrow fails here
   * as well as in `composed-contrast.test.ts`.
   */
  it("draws its state border at full strength", () => {
    for (const variant of ["default", "destructive", "copper"] as const) {
      const classes = mount(Badge, { props: { variant }, slots: { default: "x" } }).classes();
      expect(
        classes.filter((c) => /^border-\w+\//.test(c)),
        variant,
      ).toEqual([]);
    }
  });

  it("merges a custom class with the variant classes", () => {
    const wrapper = mount(Badge, {
      props: { class: "my-custom-class" },
      slots: { default: "Tag" },
    });
    expect(wrapper.classes()).toContain("my-custom-class");
    expect(wrapper.classes()).toContain("border-primary");
  });
});
