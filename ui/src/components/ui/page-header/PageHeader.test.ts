import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import { PageHeader } from ".";

describe("PageHeader", () => {
  it("renders the title and description", () => {
    const wrapper = mount(PageHeader, { props: { title: "Health", description: "Live status" } });
    expect(wrapper.find("h1").text()).toBe("Health");
    expect(wrapper.text()).toContain("Live status");
  });

  it("uses the plain style by default", () => {
    const wrapper = mount(PageHeader, { props: { title: "Health" } });
    expect(wrapper.find("h1").classes()).toContain("text-2xl");
    expect(wrapper.find("h1").classes()).not.toContain("cyber-text-glow");
  });

  it("applies the glow variant", () => {
    const wrapper = mount(PageHeader, { props: { title: "Health", variant: "glow" } });
    expect(wrapper.find("h1").classes()).toContain("cyber-text-glow");
  });

  it("renders the actions slot", () => {
    const wrapper = mount(PageHeader, {
      props: { title: "Health" },
      slots: { actions: "<button>Refresh</button>" },
    });
    expect(wrapper.text()).toContain("Refresh");
  });

  it("renders no button when the actions slot is absent", () => {
    const wrapper = mount(PageHeader, { props: { title: "Health" } });
    expect(wrapper.find("button").exists()).toBe(false);
  });

  it("renders the title slot instead of the title prop when provided", () => {
    const wrapper = mount(PageHeader, {
      slots: { title: '<svg class="icon" /> Custom Title' },
    });
    expect(wrapper.find("h1").text()).toContain("Custom Title");
    expect(wrapper.find("h1 svg.icon").exists()).toBe(true);
  });
});
