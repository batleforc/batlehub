import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import { createRouter, createMemoryHistory, type Router } from "vue-router";
import SectionTabs from "./SectionTabs.vue";

const TABS = [
  { to: "/admin/security/users", label: "Users" },
  { to: "/admin/security/ip-blocks", label: "IP Blocks" },
];

function makeRouter(initialPath: string): Router {
  const stub = { template: "<div />" };
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: "/admin/security/users", component: stub },
      { path: "/admin/security/ip-blocks", component: stub },
    ],
  });
  router.push(initialPath);
  return router;
}

describe("SectionTabs", () => {
  it("renders a link per tab", async () => {
    const router = makeRouter("/admin/security/users");
    await router.isReady();
    const wrapper = mount(SectionTabs, {
      props: { tabs: TABS },
      global: { plugins: [router] },
    });
    const links = wrapper.findAll("a");
    expect(links).toHaveLength(2);
    expect(links[0].text()).toBe("Users");
    expect(links[1].text()).toBe("IP Blocks");
  });

  it("marks the tab matching the current route as active", async () => {
    const router = makeRouter("/admin/security/ip-blocks");
    await router.isReady();
    const wrapper = mount(SectionTabs, {
      props: { tabs: TABS },
      global: { plugins: [router] },
    });
    const links = wrapper.findAll("a");
    expect(links[1].classes()).toContain("border-primary");
    expect(links[0].classes()).not.toContain("border-primary");
  });
});
