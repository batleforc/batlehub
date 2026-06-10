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
    expect(wrapper.classes()).toContain("bg-primary/10");
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
