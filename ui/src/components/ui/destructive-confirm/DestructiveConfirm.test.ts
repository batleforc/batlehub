import { mount, type VueWrapper } from "@vue/test-utils";
import { afterEach, describe, expect, it } from "vitest";
import { nextTick } from "vue";

import DestructiveConfirm from "./DestructiveConfirm.vue";

/**
 * These tests are the destructive-action contract (RFC 0003 §4.5). The point is
 * not that the dialog renders — it is that a caller *cannot* produce a bare
 * "Are you sure?" for an irreversible action on 47 objects.
 */
const base = {
  open: true,
  action: "Delete",
  count: 47,
  itemNoun: "version",
  itemNounPlural: "versions",
  scope: "internal/auth across 2 registries",
} as const;

/**
 * radix-vue portals the content out of the wrapper, so assertions read the
 * document. That makes teardown load-bearing: without it the previous test's
 * portal is still in the body and every assertion passes against stale markup.
 */
let active: VueWrapper | null = null;

afterEach(() => {
  active?.unmount();
  active = null;
  document.body.innerHTML = "";
});

async function mountOpen(props: Record<string, unknown> = {}) {
  active = mount(DestructiveConfirm, {
    props: { ...base, ...props },
    attachTo: document.body,
  });
  await nextTick();
  return active;
}

const buttonLabelled = (label: string): HTMLButtonElement =>
  [...document.querySelectorAll("button")].find((b) => b.textContent?.trim() === label)!;

describe("DestructiveConfirm", () => {
  it("names the scope and the count before the verb", async () => {
    await mountOpen();
    expect(document.body.textContent).toContain(
      "Delete 47 versions of internal/auth across 2 registries",
    );
  });

  /**
   * The dialog used to append an `s`. Every caller passes a translated noun, so
   * that rule was English morphology applied to `paquet`, `modification` and
   * `artefact en cache` — right by luck for those three and wrong for the next
   * noun added. The plural now comes from the catalogue with the singular, and
   * a caller that omits it gets the singular back rather than an invented word.
   */
  it("does not invent a plural for a noun it was not given one for", async () => {
    await mountOpen({ itemNoun: "cheval", itemNounPlural: undefined });
    expect(document.body.textContent).toContain("Delete 47 cheval");
    expect(document.body.textContent).not.toContain("chevals");
  });

  it("uses the plural the caller supplied", async () => {
    await mountOpen({ itemNoun: "cheval", itemNounPlural: "chevaux" });
    expect(document.body.textContent).toContain("Delete 47 chevaux");
  });

  it("says plainly that an irreversible action cannot be undone", async () => {
    await mountOpen();
    expect(document.body.textContent).toContain("cannot be undone");
  });

  it("says a reversible action can be undone instead", async () => {
    await mountOpen({ action: "Yank", reversible: true });
    expect(document.body.textContent).toContain("can be undone");
    expect(document.body.textContent).not.toContain("cannot be undone");
  });

  it("singularises the noun for a single object", async () => {
    await mountOpen({ count: 1 });
    expect(document.body.textContent).toContain("Delete 1 version of");
  });

  /**
   * The friction rule: irreversible actions require the typed name, reversible
   * ones do not. Uniform friction trains people to type through the prompt.
   */
  it("blocks confirmation of an irreversible action until the name is typed", async () => {
    const wrapper = await mountOpen({ confirmName: "internal/auth" });
    const button = buttonLabelled("Delete");
    expect(button.hasAttribute("disabled")).toBe(true);

    const input = document.querySelector("#destructive-confirm-name") as HTMLInputElement;
    input.value = "internal/auth";
    input.dispatchEvent(new Event("input"));
    await wrapper.vm.$nextTick();

    expect(button.hasAttribute("disabled")).toBe(false);
    expect(wrapper.emitted("confirm")).toBeUndefined();
  });

  it("does not demand a typed name for a reversible action", async () => {
    await mountOpen({ action: "Yank", reversible: true, confirmName: "internal/auth" });
    expect(document.querySelector("#destructive-confirm-name")).toBeNull();
  });

  it("refuses to act on an empty selection", async () => {
    await mountOpen({ count: 0, reversible: true });
    expect(buttonLabelled("Delete").hasAttribute("disabled")).toBe(true);
  });

  it("emits confirm when the contract is satisfied", async () => {
    const wrapper = await mountOpen({ action: "Yank", reversible: true });
    buttonLabelled("Yank").click();
    await wrapper.vm.$nextTick();
    expect(wrapper.emitted("confirm")).toHaveLength(1);
  });

  it("surfaces a failure as an alert rather than a silent no-op", async () => {
    await mountOpen({ error: "Two versions are protected by a retention policy." });
    const alert = document.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain("retention policy");
  });
});

/**
 * Every irreversible action states *its own* consequence.
 *
 * `destructive.cannotUndo` used to read "The artifacts and their metadata are
 * removed permanently", and three of the four irreversible verbs in the console
 * inherited it while removing no artifact at all: a revoked token, a forced
 * config reload and an audit-log purge. The stock sentence is the generic truth
 * now, and the specific one is the caller's to supply. Vue's `defineProps`
 * cannot express "required when another prop is false", so what actually
 * requires it is a scan over the call sites — in `system-rules.test.ts`, with
 * the other rules about source.
 */
describe("the consequence of an irreversible action", () => {
  it("falls back to a sentence that is true of any irreversible action", async () => {
    await mountOpen({ reversible: false, consequence: undefined });
    // Not "the artifacts are removed permanently" — that is a delete, and it
    // was being said over three actions that remove nothing.
    expect(document.body.textContent).toContain("cannot be undone");
    expect(document.body.textContent).not.toMatch(/artifacts/i);
  });

  it("prefers the caller's sentence when there is one", async () => {
    await mountOpen({ reversible: false, consequence: "Every CI token stops working." });
    expect(document.body.textContent).toContain("Every CI token stops working.");
    expect(document.body.textContent).not.toContain("cannot be undone");
  });
});
