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
