import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import { h } from "vue";
import Input from "./Input.vue";
import Label from "../label/Label.vue";

/**
 * The accessible name, by the subset of the accname algorithm that applies to
 * an `<input>`: `aria-labelledby`, then `aria-label`, then a `<label for>`,
 * then — for nothing else in this component — the placeholder.
 *
 * The order is the whole point of these tests. `aria-label` outranks
 * `<label for>`, which is why binding it unconditionally silenced every real
 * label in the console rather than supplementing it.
 */
function accessibleName(input: HTMLInputElement): string | null {
  const labelledby = input.getAttribute("aria-labelledby");
  if (labelledby) return document.getElementById(labelledby)?.textContent?.trim() ?? null;

  const label = input.getAttribute("aria-label");
  if (label) return label;

  const forLabel = input.id ? document.querySelector(`label[for="${input.id}"]`) : null;
  if (forLabel) return forLabel.textContent?.trim() ?? null;

  return input.getAttribute("placeholder");
}

describe("Input", () => {
  it("renders an input element with base classes", () => {
    const wrapper = mount(Input);
    expect(wrapper.element.tagName).toBe("INPUT");
    expect(wrapper.classes()).toContain("flex");
    expect(wrapper.classes()).toContain("h-9");
  });

  it("passes through type, placeholder and disabled", () => {
    const wrapper = mount(Input, {
      props: { type: "password", placeholder: "Token", disabled: true },
    });
    expect(wrapper.attributes("type")).toBe("password");
    expect(wrapper.attributes("placeholder")).toBe("Token");
    expect(wrapper.attributes("disabled")).toBeDefined();
  });

  it("displays the modelValue", () => {
    const wrapper = mount(Input, { props: { modelValue: "hello" } });
    expect((wrapper.element as HTMLInputElement).value).toBe("hello");
  });

  it("emits update:modelValue on input", async () => {
    const wrapper = mount(Input, { props: { modelValue: "" } });
    const input = wrapper.find("input");
    await input.setValue("new value");
    expect(wrapper.emitted("update:modelValue")?.[0]).toEqual(["new value"]);
  });

  it("merges a custom class with the base classes", () => {
    const wrapper = mount(Input, { props: { class: "my-input" } });
    expect(wrapper.classes()).toContain("my-input");
    expect(wrapper.classes()).toContain("flex");
  });

  /**
   * `:aria-label="placeholder"` was bound with no condition at all, and
   * `aria-label` wins the accessible name computation over `<label for>`. So a
   * field with a correct, translated, visible label announced its placeholder
   * instead — `"CVE-2025-XXXX or policy violation"` on the bulk page,
   * `"e.g. CI pipeline"` on the tokens page, both in English to a French
   * screen-reader user.
   *
   * WCAG 2.5.3 Label in Name (A) and 3.3.2 Labels or Instructions (A).
   */
  describe("accessible name", () => {
    it("is the label, not the placeholder, when the field is labelled", () => {
      const wrapper = mount(
        {
          render: () =>
            h("div", [
              h(Label, { for: "token-name" }, () => "Nom"),
              h(Input, { id: "token-name", placeholder: "e.g. CI pipeline" }),
            ]),
        },
        { attachTo: document.body },
      );

      const input = wrapper.find("input").element as HTMLInputElement;
      expect(input.getAttribute("aria-label")).toBeNull();
      expect(accessibleName(input)).toBe("Nom");
      wrapper.unmount();
    });

    it("yields to an aria-labelledby", () => {
      const wrapper = mount(Input, {
        props: { ariaLabelledby: "heading", placeholder: "e.g. CI pipeline" },
      });
      const input = wrapper.element as HTMLInputElement;
      expect(input.getAttribute("aria-labelledby")).toBe("heading");
      expect(input.getAttribute("aria-label")).toBeNull();
    });

    it("yields to an aria-label the caller supplied", () => {
      const wrapper = mount(Input, {
        attrs: { "aria-label": "Recherche" },
        props: { placeholder: "e.g. CI pipeline" },
      });
      expect(wrapper.attributes("aria-label")).toBe("Recherche");
    });

    it("still names an unlabelled field by its placeholder", () => {
      // The fallback is kept: a placeholder is a poor name and no name is
      // worse. It is the *override* that was the defect, not the fallback.
      const wrapper = mount(Input, { props: { placeholder: "Filtrer" } });
      expect(wrapper.attributes("aria-label")).toBe("Filtrer");
    });

    it("renders the id it was given, so a label can bind to it", () => {
      const wrapper = mount(Input, { props: { id: "token-name" } });
      expect(wrapper.attributes("id")).toBe("token-name");
      expect(wrapper.attributes("arialabelledby")).toBeUndefined();
    });
  });

  /**
   * DESIGN.md specifies the placeholder in `--ink-dim` and says it carries a
   * real example of the expected input — content, not ornament. The extra
   * `/60` composited it to 2.59:1 in dark and 2.84:1 in light, both under the
   * 4.5:1 floor. `composed-contrast.test.ts` reads this class string, so the
   * two cannot drift apart.
   */
  it("paints the placeholder at full --ink-dim", () => {
    const wrapper = mount(Input);
    expect(wrapper.classes()).toContain("placeholder:text-muted-foreground");
  });
});
