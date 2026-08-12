import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import Facet from "./Facet.vue";

const options = [
  { value: "npm1", label: "npm1", count: 1284 },
  { value: "cargo1", label: "cargo1", count: 892 },
];

describe("Facet", () => {
  it("marks the selected option with aria-pressed", () => {
    const w = mount(Facet, { props: { modelValue: "npm1", options, label: "Registries" } });
    const buttons = w.findAll("button");
    expect(buttons[0].attributes("aria-pressed")).toBe("true");
    expect(buttons[1].attributes("aria-pressed")).toBe("false");
  });

  it("emits the chosen value", async () => {
    const w = mount(Facet, { props: { modelValue: "npm1", options, label: "Registries" } });
    await w.findAll("button")[1].trigger("click");
    expect(w.emitted("update:modelValue")).toEqual([["cargo1"]]);
  });

  it("emits null for the all option", async () => {
    const w = mount(Facet, {
      props: { modelValue: "npm1", options, label: "Registries", allLabel: "All registries" },
    });
    await w.findAll("button")[0].trigger("click");
    expect(w.emitted("update:modelValue")).toEqual([[null]]);
  });

  /**
   * Selection must not rest on the fill: at this palette's lightness a fill step
   * measures ~1.06:1, invisible to many users while looking fine to the author.
   * Ink and a lit edge carry it (The Undependable Fill Rule).
   */
  it("carries selection on ink and a lit edge, not on fill alone", () => {
    const w = mount(Facet, { props: { modelValue: "npm1", options, label: "Registries" } });
    const selected = w.findAll("button")[0];
    expect(selected.classes()).toContain("border-l-primary");
    expect(selected.classes()).toContain("text-foreground");
    expect(selected.classes()).toContain("font-semibold");
  });

  it("labels the group and formats counts for the reader's locale", () => {
    const w = mount(Facet, { props: { modelValue: null, options, label: "Registries" } });
    expect(w.find("ul").attributes("aria-labelledby")).toBe("facet-Registries");
    expect(w.text()).toContain(new Intl.NumberFormat().format(1284));
  });
});
