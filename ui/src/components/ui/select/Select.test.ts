import { describe, it, expect, afterEach } from "vitest";
import { mount } from "@vue/test-utils";
import Select from "./Select.vue";

const options = [
  { value: "a", label: "Option A" },
  { value: "b", label: "Option B" },
];

describe("Select", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("renders the trigger with placeholder and base classes", () => {
    const wrapper = mount(Select, {
      props: { options, placeholder: "Choose…" },
      attachTo: document.body,
    });
    expect(wrapper.text()).toContain("Choose…");
    const trigger = wrapper.find('[role="combobox"]');
    expect(trigger.classes()).toContain("flex");
    expect(trigger.classes()).toContain("rounded-md");
  });

  it("passes the id prop to the trigger", () => {
    const wrapper = mount(Select, {
      props: { options, id: "registry-select" },
      attachTo: document.body,
    });
    expect(wrapper.find('[role="combobox"]').attributes("id")).toBe("registry-select");
  });

  it("merges a custom class onto the trigger", () => {
    const wrapper = mount(Select, {
      props: { options, class: "my-select" },
      attachTo: document.body,
    });
    expect(wrapper.find('[role="combobox"]').classes()).toContain("my-select");
  });

  it("opens and lists options when the trigger is activated", async () => {
    const wrapper = mount(Select, {
      props: { options, placeholder: "Choose…" },
      attachTo: document.body,
    });

    // radix-vue opens the select on a left-button, non-touch pointerdown.
    wrapper.find('[role="combobox"]').element.dispatchEvent(
      new PointerEvent("pointerdown", {
        bubbles: true,
        cancelable: true,
        button: 0,
        ctrlKey: false,
        pointerType: "mouse",
      }),
    );
    await wrapper.vm.$nextTick();
    await wrapper.vm.$nextTick();
    await wrapper.vm.$nextTick();

    const items = document.body.querySelectorAll('[role="option"]');
    expect(items).toHaveLength(2);
    expect(document.body.textContent).toContain("Option A");
    expect(document.body.textContent).toContain("Option B");
  });
});
