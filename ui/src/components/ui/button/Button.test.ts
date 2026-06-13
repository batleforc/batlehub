import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import { Button } from ".";

describe("Button", () => {
  it("renders slot content", () => {
    const wrapper = mount(Button, { slots: { default: "Click me" } });
    expect(wrapper.text()).toBe("Click me");
    expect(wrapper.element.tagName).toBe("BUTTON");
  });

  it("applies the default variant and size classes", () => {
    const wrapper = mount(Button, { slots: { default: "Go" } });
    expect(wrapper.classes()).toContain("bg-primary");
    expect(wrapper.classes()).toContain("h-9");
  });

  it("applies variant and size props", () => {
    const wrapper = mount(Button, {
      props: { variant: "destructive", size: "sm" },
      slots: { default: "Delete" },
    });
    expect(wrapper.classes()).toContain("bg-destructive");
    expect(wrapper.classes()).toContain("h-8");
  });

  it("disables the button when the disabled prop is set", () => {
    const wrapper = mount(Button, { props: { disabled: true }, slots: { default: "Go" } });
    expect(wrapper.attributes("disabled")).toBeDefined();
  });

  it("forwards extra attributes such as click handlers", async () => {
    let clicked = 0;
    const wrapper = mount(Button, {
      attrs: { onClick: () => clicked++ },
      slots: { default: "Go" },
    });
    await wrapper.trigger("click");
    expect(clicked).toBe(1);
  });

  it("merges a custom class with the variant classes", () => {
    const wrapper = mount(Button, {
      props: { class: "my-custom-class" },
      slots: { default: "Go" },
    });
    expect(wrapper.classes()).toContain("my-custom-class");
    expect(wrapper.classes()).toContain("bg-primary");
  });
});
