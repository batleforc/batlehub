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
    expect(wrapper.classes()).toContain("border-primary/40");
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
    expect(wrapper.classes()).toContain("border-destructive/40");
    expect(wrapper.classes()).toContain("text-destructive");
  });

  it("merges a custom class with the variant classes", () => {
    const wrapper = mount(Badge, {
      props: { class: "my-custom-class" },
      slots: { default: "Tag" },
    });
    expect(wrapper.classes()).toContain("my-custom-class");
    expect(wrapper.classes()).toContain("border-primary/40");
  });
});
