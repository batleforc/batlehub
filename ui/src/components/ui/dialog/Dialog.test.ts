import { describe, it, expect, afterEach } from "vitest";
import { mount } from "@vue/test-utils";
import Dialog from "./Dialog.vue";

describe("Dialog", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("does not render content when closed", () => {
    mount(Dialog, {
      props: { open: false },
      slots: { default: "Dialog body" },
      attachTo: document.body,
    });
    expect(document.body.textContent).not.toContain("Dialog body");
  });

  it("renders the slot content in a portal when open", async () => {
    const wrapper = mount(Dialog, {
      props: { open: true },
      slots: { default: "Dialog body" },
      attachTo: document.body,
    });
    await wrapper.vm.$nextTick();
    await wrapper.vm.$nextTick();

    expect(document.body.textContent).toContain("Dialog body");
    expect(document.body.querySelector('[role="dialog"]')).not.toBeNull();
  });

  it("renders the trigger slot inline (not in a portal)", () => {
    const wrapper = mount(Dialog, {
      props: { open: false },
      slots: { trigger: "<button>Open</button>" },
      attachTo: document.body,
    });
    expect(wrapper.text()).toContain("Open");
  });

  it("emits update:open false when the close button is clicked", async () => {
    const wrapper = mount(Dialog, {
      props: { open: true },
      slots: { default: "Dialog body" },
      attachTo: document.body,
    });
    await wrapper.vm.$nextTick();
    await wrapper.vm.$nextTick();

    const closeButton = document.body.querySelector('[role="dialog"] button') as HTMLElement;
    expect(closeButton).not.toBeNull();
    closeButton.click();
    await wrapper.vm.$nextTick();

    expect(wrapper.emitted("update:open")?.[0]).toEqual([false]);
  });
});
