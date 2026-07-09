import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import RegistryPathResults from "./RegistryPathResults.vue";

describe("RegistryPathResults", () => {
  it("shows a placeholder message when there are no paths", () => {
    const wrapper = mount(RegistryPathResults, { props: { paths: [], baseUrl: "https://host" } });
    expect(wrapper.text()).toContain("Fill in the fields above");
  });

  it("renders each path with the base url prefixed", () => {
    const wrapper = mount(RegistryPathResults, {
      props: {
        paths: [{ label: "Metadata", url: "/proxy/npm/foo", available: true }],
        baseUrl: "https://host",
      },
    });
    expect(wrapper.text()).toContain("Metadata");
    expect(wrapper.find("code").text()).toContain("https://host/proxy/npm/foo");
  });

  it("shows a copy button for available paths and a badge for unavailable ones", () => {
    const wrapper = mount(RegistryPathResults, {
      props: {
        paths: [
          { label: "Ready", url: "/proxy/npm/a", available: true },
          { label: "Missing fields", url: "/proxy/npm/b", available: false },
        ],
        baseUrl: "https://host",
      },
    });
    const rows = wrapper.findAll(".divide-y > div");
    expect(rows[0].find("button").exists()).toBe(true);
    expect(rows[1].text()).toContain("needs more fields");
  });
});
