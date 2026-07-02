import { describe, it, expect, afterEach } from "vitest";
import { mount } from "@vue/test-utils";
import ConfirmDialog from "./ConfirmDialog.vue";

describe("ConfirmDialog", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("renders title, description, and error when open", async () => {
    const wrapper = mount(ConfirmDialog, {
      props: {
        open: true,
        title: "Clear cache?",
        description: "This cannot be undone.",
        error: "Something went wrong",
      },
      attachTo: document.body,
    });
    await wrapper.vm.$nextTick();
    await wrapper.vm.$nextTick();

    expect(document.body.textContent).toContain("Clear cache?");
    expect(document.body.textContent).toContain("This cannot be undone.");
    expect(document.body.textContent).toContain("Something went wrong");
  });

  it("emits update:open false when cancel is clicked", async () => {
    const wrapper = mount(ConfirmDialog, {
      props: { open: true, title: "Clear cache?" },
      attachTo: document.body,
    });
    await wrapper.vm.$nextTick();
    await wrapper.vm.$nextTick();

    const buttons = Array.from(document.body.querySelectorAll('[role="dialog"] button'));
    const cancelButton = buttons.find((b) => b.textContent?.trim() === "Cancel") as HTMLElement;
    expect(cancelButton).toBeTruthy();
    cancelButton.click();
    await wrapper.vm.$nextTick();

    expect(wrapper.emitted("update:open")?.[0]).toEqual([false]);
  });

  it("emits confirm when the confirm button is clicked", async () => {
    const wrapper = mount(ConfirmDialog, {
      props: { open: true, title: "Clear cache?", confirmLabel: "Clear Cache" },
      attachTo: document.body,
    });
    await wrapper.vm.$nextTick();
    await wrapper.vm.$nextTick();

    const buttons = Array.from(document.body.querySelectorAll('[role="dialog"] button'));
    const confirmButton = buttons.find(
      (b) => b.textContent?.trim() === "Clear Cache",
    ) as HTMLElement;
    expect(confirmButton).toBeTruthy();
    confirmButton.click();

    expect(wrapper.emitted("confirm")).toBeTruthy();
  });

  it("renders the title slot instead of the title prop when provided", async () => {
    const wrapper = mount(ConfirmDialog, {
      props: { open: true },
      slots: { title: '<span class="font-mono">registry-a</span>?' },
      attachTo: document.body,
    });
    await wrapper.vm.$nextTick();
    await wrapper.vm.$nextTick();

    expect(document.body.querySelector('[role="dialog"] span.font-mono')?.textContent).toBe(
      "registry-a",
    );
  });

  it("shows the loading label and disables buttons while loading", async () => {
    const wrapper = mount(ConfirmDialog, {
      props: {
        open: true,
        title: "Clear cache?",
        confirmLabel: "Clear Cache",
        loadingLabel: "Clearing…",
        loading: true,
      },
      attachTo: document.body,
    });
    await wrapper.vm.$nextTick();
    await wrapper.vm.$nextTick();

    expect(document.body.textContent).toContain("Clearing…");
    const buttons = Array.from(
      document.body.querySelectorAll('[role="dialog"] button'),
    ) as HTMLButtonElement[];
    const cancelButton = buttons.find((b) => b.textContent?.trim() === "Cancel");
    const confirmButton = buttons.find((b) => b.textContent?.trim() === "Clearing…");
    expect(cancelButton?.disabled).toBe(true);
    expect(confirmButton?.disabled).toBe(true);
  });
});
