import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import { AsyncState } from ".";

describe("AsyncState", () => {
  it("renders the loading slot when loading", () => {
    const wrapper = mount(AsyncState, {
      props: { loading: true },
      slots: { default: "Content" },
    });
    expect(wrapper.text()).toContain("Loading…");
    expect(wrapper.text()).not.toContain("Content");
  });

  it("renders the error state when not loading and an error is set", () => {
    const wrapper = mount(AsyncState, {
      props: { loading: false, error: "Boom" },
      slots: { default: "Content" },
    });
    expect(wrapper.text()).toContain("Boom");
    expect(wrapper.text()).not.toContain("Content");
  });

  it("renders the empty state when not loading, no error, and empty", () => {
    const wrapper = mount(AsyncState, {
      props: { loading: false, empty: true, emptyMessage: "Nothing here" },
      slots: { default: "Content" },
    });
    expect(wrapper.text()).toContain("Nothing here");
    expect(wrapper.text()).not.toContain("Content");
  });

  it("renders the default slot once loaded, error-free, and non-empty", () => {
    const wrapper = mount(AsyncState, {
      props: { loading: false },
      slots: { default: "Content" },
    });
    expect(wrapper.text()).toBe("Content");
  });

  it("allows overriding the loading/error/empty slots", () => {
    const wrapper = mount(AsyncState, {
      props: { loading: true },
      slots: { loading: "Custom loading" },
    });
    expect(wrapper.text()).toBe("Custom loading");
  });
});
