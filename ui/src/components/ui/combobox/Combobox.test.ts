import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";

import Combobox from "./Combobox.vue";

const OPTIONS = [
  { value: "lodash" },
  { value: "left-pad" },
  { value: "oidc:alice", label: "alice", hint: "audit" },
];

function make(props: Record<string, unknown> = {}) {
  return mount(Combobox, {
    props: { id: "cb", modelValue: "", options: OPTIONS, ...props },
  });
}

const input = (w: ReturnType<typeof make>) => w.find("input");
const options = (w: ReturnType<typeof make>) => w.findAll('[role="option"]');
const active = (w: ReturnType<typeof make>) => input(w).attributes("aria-activedescendant");

/**
 * The primitive `ui/` did not have (RFC 0004-bis §6.2).
 *
 * Two pages needed one and improvised with `<datalist>`, which cannot be styled
 * to the world, behaves differently per browser, exposes no listbox to
 * assistive tech, and cannot render a "no match" message — the one behaviour
 * this component exists to get. So the assertions here are the contract, and
 * the per-field tests that follow assert only which source a field is bound to.
 */
describe("Combobox", () => {
  describe("the ARIA contract", () => {
    it("is a combobox owning a listbox", () => {
      const w = make();
      expect(input(w).attributes("role")).toBe("combobox");
      expect(input(w).attributes("aria-controls")).toBe("cb-listbox");
      expect(w.find("#cb-listbox").attributes("role")).toBe("listbox");
    });

    it("reports whether the list is showing", async () => {
      const w = make();
      expect(input(w).attributes("aria-expanded")).toBe("false");
      await input(w).trigger("focus");
      expect(input(w).attributes("aria-expanded")).toBe("true");
    });

    it("tracks the highlighted option with aria-activedescendant", async () => {
      const w = make();
      await input(w).trigger("focus");
      expect(active(w)).toBeUndefined();

      await input(w).trigger("keydown", { key: "ArrowDown" });
      expect(active(w)).toBe("cb-listbox-opt-0");
      expect(options(w)[0].attributes("aria-selected")).toBe("true");
    });
  });

  describe("keyboard navigation", () => {
    it("arrows through the options", async () => {
      const w = make();
      await input(w).trigger("focus");
      await input(w).trigger("keydown", { key: "ArrowDown" });
      await input(w).trigger("keydown", { key: "ArrowDown" });
      expect(active(w)).toBe("cb-listbox-opt-1");

      await input(w).trigger("keydown", { key: "ArrowUp" });
      expect(active(w)).toBe("cb-listbox-opt-0");
    });

    it("ArrowUp from the first option returns to the typed text", async () => {
      const w = make({ modelValue: "lo" });
      await input(w).trigger("focus");
      await input(w).trigger("keydown", { key: "ArrowDown" });
      await input(w).trigger("keydown", { key: "ArrowUp" });
      // Not a jump to the bottom of a list you were leaving.
      expect(active(w)).toBeUndefined();
    });

    it("Home and End jump to the ends", async () => {
      const w = make();
      await input(w).trigger("focus");
      await input(w).trigger("keydown", { key: "End" });
      expect(active(w)).toBe("cb-listbox-opt-2");

      await input(w).trigger("keydown", { key: "Home" });
      expect(active(w)).toBe("cb-listbox-opt-0");
    });

    it("Enter takes the highlighted suggestion", async () => {
      const w = make();
      await input(w).trigger("focus");
      await input(w).trigger("keydown", { key: "ArrowDown" });
      await input(w).trigger("keydown", { key: "Enter" });

      expect(w.emitted("update:modelValue")?.at(-1)).toEqual(["lodash"]);
      expect(w.emitted("select")?.at(-1)).toEqual(["lodash"]);
    });

    /**
     * A combobox that swallows Enter turns "submit what I typed" into "nothing
     * happened" — and typing a value the source has never seen is the case
     * `AdminWarming` exists for.
     */
    it("Enter with nothing highlighted belongs to the form", async () => {
      const w = make({ modelValue: "not-cached-yet" });
      await input(w).trigger("focus");
      const event = { key: "Enter", preventDefault: () => {} };
      await input(w).trigger("keydown", event);
      expect(w.emitted("select")).toBeUndefined();
    });

    it("Escape reverts to the typed text and closes", async () => {
      const w = make({ modelValue: "lo" });
      await input(w).trigger("focus");
      await input(w).trigger("keydown", { key: "ArrowDown" });
      await input(w).trigger("keydown", { key: "Escape" });

      expect(input(w).attributes("aria-expanded")).toBe("false");
      expect(w.emitted("update:modelValue")?.at(-1)).toEqual(["lo"]);
    });
  });

  describe("what it says when it has nothing", () => {
    /**
     * "Nothing cached matches `lodahs`" is the message that catches the typo.
     * An empty popup is a blank that reads as a fact — the same defect as the
     * audit log's empty envelope and the access-check simulator's `allow`.
     */
    it("states that nothing matched, naming the query", async () => {
      const w = make({ modelValue: "lodahs", options: [] });
      await input(w).trigger("focus");
      expect(w.text()).toContain("lodahs");
      expect(w.text()).toMatch(/nothing matches/i);
    });

    it("says it is still looking rather than that nothing matched", async () => {
      const w = make({ modelValue: "lo", options: [], loading: true });
      await input(w).trigger("focus");
      expect(w.text()).toMatch(/searching/i);
      expect(w.text()).not.toMatch(/nothing matches/i);
    });

    it("announces the result count to a live region", async () => {
      const w = make();
      await input(w).trigger("focus");
      expect(w.find('[role="status"]').text()).toMatch(/3 suggestions/i);
    });

    it("states why it is disabled rather than being silently dark", () => {
      const w = make({ disabled: true, disabledReason: "Choose a package first." });
      expect(w.text()).toContain("Choose a package first.");
      expect(input(w).attributes("aria-describedby")).toBe("cb-listbox-reason");
    });
  });

  describe("typing", () => {
    it("emits what was typed, suggestion or not", async () => {
      const w = make();
      await input(w).setValue("something-nobody-has");
      expect(w.emitted("update:modelValue")?.at(-1)).toEqual(["something-nobody-has"]);
      // Typing is not selecting: the value is the operator's, not the list's.
      expect(w.emitted("select")).toBeUndefined();
    });

    it("shows a label when it differs from the value it will submit", async () => {
      const w = make();
      await input(w).trigger("focus");
      expect(options(w)[2].text()).toContain("alice");
      await options(w)[2].trigger("mousedown");
      // `oidc:alice` is what the instance stores, and what gets submitted.
      expect(w.emitted("update:modelValue")?.at(-1)).toEqual(["oidc:alice"]);
    });

    it("clicking a suggestion takes it before the blur can close the list", async () => {
      const w = make();
      await input(w).trigger("focus");
      await options(w)[1].trigger("mousedown");
      expect(w.emitted("select")?.at(-1)).toEqual(["left-pad"]);
    });
  });
});
