import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import Separator from "./Separator.vue";

describe("Separator", () => {
  it("defaults to horizontal orientation classes", () => {
    const wrapper = mount(Separator);
    expect(wrapper.classes()).toContain("h-[1px]");
    expect(wrapper.classes()).toContain("w-full");
    expect(wrapper.classes()).toContain("bg-border");
  });

  it("applies vertical orientation classes", () => {
    const wrapper = mount(Separator, { props: { orientation: "vertical" } });
    expect(wrapper.classes()).toContain("h-full");
    expect(wrapper.classes()).toContain("w-[1px]");
  });

  it("merges a custom class with the orientation classes", () => {
    const wrapper = mount(Separator, { props: { class: "my-separator" } });
    expect(wrapper.classes()).toContain("my-separator");
    expect(wrapper.classes()).toContain("bg-border");
  });
});
