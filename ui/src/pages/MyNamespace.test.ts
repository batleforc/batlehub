import { mount, flushPromises } from "@vue/test-utils";
import { describe, expect, it, vi } from "vitest";

const { myNamespacesMock, listRegistriesMock } = vi.hoisted(() => ({
  myNamespacesMock: vi.fn(),
  listRegistriesMock: vi.fn(),
}));

vi.mock("@/client/sdk.gen", () => ({
  myNamespaces: myNamespacesMock,
  listRegistries: listRegistriesMock,
}));

vi.mock("@/composables/useAuth", () => ({
  useAuth: () => ({
    token: { value: "t" },
    identity: { value: { groups: ["team-a"] } },
  }),
}));

import MyNamespace from "./MyNamespace.vue";

/**
 * WCAG 2.1.1 Keyboard (A), the same defect the catalogue's rows had.
 *
 * `<TableRow @click="selectNamespace(ns)">` carried no `tabindex`, no `role`
 * and no key handler, and this list is the *only* route to the packages table
 * further down the page — so from a keyboard, that table did not exist.
 *
 * Unlike the catalogue this is a selection rather than a navigation: a chosen
 * namespace has no URL, so the fix is a `<button>` carrying `aria-pressed`
 * rather than a `RouterLink`.
 */
describe("MyNamespace list", () => {
  const mountPage = async () => {
    myNamespacesMock.mockResolvedValue({
      data: [
        { registry: "npm", prefix: "@acme", group_id: "team-a" },
        { registry: "maven", prefix: "com.acme", group_id: "team-a" },
      ],
    });
    listRegistriesMock.mockResolvedValue({ data: [] });
    const wrapper = mount(MyNamespace, { global: { stubs: { NamespaceUpload: true } } });
    await flushPromises();
    return wrapper;
  };

  it("puts a real control on each namespace rather than on the row", async () => {
    const wrapper = await mountPage();

    const buttons = wrapper.findAll("tbody button");
    expect(buttons).toHaveLength(2);
    expect(buttons[0].text()).toBe("@acme");
    // A `<button>` is focusable and Enter/Space-activated by the browser: the
    // assertion is that the element *is* one, not that a handler was added.
    expect(buttons[0].element.tagName).toBe("BUTTON");
    expect(buttons[0].attributes("type")).toBe("button");
  });

  it("no longer dresses the row as a click target", async () => {
    const wrapper = await mountPage();
    for (const row of wrapper.findAll("tbody tr")) {
      expect(row.classes()).not.toContain("cursor-pointer");
    }
  });

  it("announces which namespace is selected", async () => {
    const wrapper = await mountPage();
    const buttons = wrapper.findAll("tbody button");
    expect(buttons[0].attributes("aria-pressed")).toBe("false");

    await buttons[0].trigger("click");
    expect(buttons[0].attributes("aria-pressed")).toBe("true");
    expect(buttons[1].attributes("aria-pressed")).toBe("false");
  });

  it("selecting a namespace reveals its packages", async () => {
    const wrapper = await mountPage();
    expect(wrapper.findComponent({ name: "NamespacePackagesTable" }).exists()).toBe(false);

    await wrapper.findAll("tbody button")[0].trigger("click");
    expect(wrapper.findComponent({ name: "NamespacePackagesTable" }).exists()).toBe(true);
  });
});
