import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";

import ConfigReadOnlyView from "./ConfigReadOnlyView.vue";

const TOML = '[server]\nport = 8080\n\n[[registries]]\nname = "npm"\n';

const mountView = (props: { content: string; error?: string | null }) =>
  mount(ConfigReadOnlyView, { props });

/**
 * RFC 0003 §6.4: when the config is mounted read-only this is a *different*
 * screen, not a disabled editor. So the assertions are about what it is not —
 * no textarea, no Apply — as much as about what it shows.
 */
describe("ConfigReadOnlyView", () => {
  it("shows the running config without offering to edit it", () => {
    const wrapper = mountView({ content: TOML });
    expect(wrapper.text()).toContain("port = 8080");
    expect(wrapper.find("textarea").exists()).toBe(false);
    expect(wrapper.findAll("button").some((b) => /apply|validate/i.test(b.text()))).toBe(false);
  });

  it("says where the config actually comes from", () => {
    const text = mountView({ content: TOML }).text();
    expect(text).toContain("read-only");
    expect(text).toContain("ConfigMap");
  });

  it("offers the config for copying, since it cannot be edited here", () => {
    const wrapper = mountView({ content: TOML });
    const copy = wrapper.findAll("button").find((b) => /copy/i.test(b.text() + b.html()));
    expect(copy).toBeDefined();
  });

  it("shows the load error instead of an empty config block", () => {
    const wrapper = mountView({ content: "", error: "permission denied reading config.toml" });
    expect(wrapper.text()).toContain("permission denied reading config.toml");
    expect(wrapper.find("pre").exists()).toBe(false);
  });
});
