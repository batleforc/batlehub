import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import RegistryPathForm from "./RegistryPathForm.vue";
import { REGISTRY_PATH_TYPES } from "@/config/registryPathFields";

function mountForm(typeId: string, values: Record<string, string> = {}) {
  const typeDef = REGISTRY_PATH_TYPES.find((t) => t.id === typeId)!;
  const allValues = Object.fromEntries(
    typeDef.fields.map((f) => [f.key, values[f.key] ?? f.default ?? ""]),
  );
  return mount(RegistryPathForm, {
    props: {
      typeDef,
      registries: [],
      registryName: typeDef.id,
      values: allValues,
      "onUpdate:registryName": () => {},
      "onUpdate:values": () => {},
    },
  });
}

describe("RegistryPathForm", () => {
  it("renders a registry-name input and one input per field", () => {
    const wrapper = mountForm("npm");
    expect(wrapper.find("#npm-registry").exists()).toBe(true);
    expect(wrapper.find("#npm-package").exists()).toBe(true);
    expect(wrapper.find("#npm-version").exists()).toBe(true);
  });

  it("groups fields sharing a row number into one grid container", () => {
    const wrapper = mountForm("github");
    // owner+repo share row 1, ref+assetId share row 2 -> two grid-cols-2 rows.
    const grids = wrapper.findAll(".grid-cols-2");
    expect(grids.length).toBe(2);
  });

  it("renders a 3-column grid for terraform's namespace/name/provider row", () => {
    const wrapper = mountForm("terraform");
    expect(wrapper.findAll(".grid-cols-3").length).toBe(2);
  });

  it("renders the suffix text next to the label", () => {
    const wrapper = mountForm("npm");
    expect(wrapper.text()).toContain("(optional)");
  });

  it("renders the note as trusted HTML", () => {
    const wrapper = mountForm("maven");
    expect(wrapper.find("code").exists()).toBe(true);
  });
});
