import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { h } from "vue";
import { mount, flushPromises } from "@vue/test-utils";
import { CopyButton } from ".";

describe("CopyButton", () => {
  const writeText = vi.fn().mockResolvedValue(undefined);

  beforeEach(() => {
    vi.useFakeTimers();
    writeText.mockClear();
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders the default label", () => {
    const wrapper = mount(CopyButton, { props: { text: "hello" } });
    expect(wrapper.text()).toBe("Copy");
  });

  it("copies the text and flips to the copied label, then resets", async () => {
    const wrapper = mount(CopyButton, { props: { text: "hello", resetMs: 1000 } });
    await wrapper.find("button").trigger("click");
    await flushPromises();
    expect(writeText).toHaveBeenCalledWith("hello");
    expect(wrapper.text()).toBe("Copied!");

    vi.advanceTimersByTime(1000);
    await wrapper.vm.$nextTick();
    expect(wrapper.text()).toBe("Copy");
  });

  it("emits copied on click", async () => {
    const wrapper = mount(CopyButton, { props: { text: "hello" } });
    await wrapper.find("button").trigger("click");
    await flushPromises();
    expect(wrapper.emitted("copied")).toBeTruthy();
  });

  it("supports custom labels", () => {
    const wrapper = mount(CopyButton, {
      props: { text: "hello", label: "Copy URL", copiedLabel: "Done" },
    });
    expect(wrapper.text()).toBe("Copy URL");
  });

  it("exposes the copied state to a scoped default slot", async () => {
    const wrapper = mount(CopyButton, {
      props: { text: "hello" },
      slots: { default: (scope: { copied: boolean }) => h("span", scope.copied ? "yes" : "no") },
    });
    expect(wrapper.text()).toBe("no");
    await wrapper.find("button").trigger("click");
    await flushPromises();
    expect(wrapper.text()).toBe("yes");
  });
});
