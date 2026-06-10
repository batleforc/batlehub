import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import Input from "./Input.vue";

describe("Input", () => {
  it("renders an input element with base classes", () => {
    const wrapper = mount(Input);
    expect(wrapper.element.tagName).toBe("INPUT");
    expect(wrapper.classes()).toContain("flex");
    expect(wrapper.classes()).toContain("h-9");
  });

  it("passes through type, placeholder and disabled", () => {
    const wrapper = mount(Input, {
      props: { type: "password", placeholder: "Token", disabled: true },
    });
    expect(wrapper.attributes("type")).toBe("password");
    expect(wrapper.attributes("placeholder")).toBe("Token");
    expect(wrapper.attributes("disabled")).toBeDefined();
  });

  it("displays the modelValue", () => {
    const wrapper = mount(Input, { props: { modelValue: "hello" } });
    expect((wrapper.element as HTMLInputElement).value).toBe("hello");
  });

  it("emits update:modelValue on input", async () => {
    const wrapper = mount(Input, { props: { modelValue: "" } });
    const input = wrapper.find("input");
    await input.setValue("new value");
    expect(wrapper.emitted("update:modelValue")?.[0]).toEqual(["new value"]);
  });

  it("merges a custom class with the base classes", () => {
    const wrapper = mount(Input, { props: { class: "my-input" } });
    expect(wrapper.classes()).toContain("my-input");
    expect(wrapper.classes()).toContain("flex");
  });
});
