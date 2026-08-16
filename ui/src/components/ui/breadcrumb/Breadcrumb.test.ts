import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import Breadcrumb from "./Breadcrumb.vue";

const stubs = { RouterLink: { props: ["to"], template: "<a :href='to'><slot/></a>" } };
const items = [
  { label: "Packages", to: "/packages" },
  { label: "npm1", to: "/packages?registry=npm1" },
  { label: "@batlehub/ui-kit" },
];

describe("Breadcrumb", () => {
  it("is a labelled navigation landmark", () => {
    const w = mount(Breadcrumb, { props: { items }, global: { stubs } });
    expect(w.element.tagName).toBe("NAV");
    expect(w.attributes("aria-label")).toBe("Breadcrumb");
  });

  it("marks the last entry as the current page and does not link it", () => {
    const w = mount(Breadcrumb, { props: { items }, global: { stubs } });
    const current = w.find('[aria-current="page"]');
    expect(current.text()).toBe("@batlehub/ui-kit");
    expect(current.element.tagName).not.toBe("A");
    expect(w.findAll("a")).toHaveLength(2);
  });

  /** Separators are decoration; read aloud they interrupt every label. */
  it("hides the separators from assistive tech", () => {
    const w = mount(Breadcrumb, { props: { items }, global: { stubs } });
    const seps = w.findAll('[aria-hidden="true"]');
    expect(seps).toHaveLength(items.length - 1);
    expect(seps[0].text()).toBe("/");
  });

  it("uses an ordered list, because the order is the meaning", () => {
    const w = mount(Breadcrumb, { props: { items }, global: { stubs } });
    expect(w.find("ol").exists()).toBe(true);
    expect(w.findAll("li")).toHaveLength(3);
  });
});
