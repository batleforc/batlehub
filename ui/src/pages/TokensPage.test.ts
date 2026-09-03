import { mount, flushPromises } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { listTokensMock, createTokenMock, revokeTokenMock } = vi.hoisted(() => ({
  listTokensMock: vi.fn(),
  createTokenMock: vi.fn(),
  revokeTokenMock: vi.fn(),
}));

vi.mock("@/client/sdk.gen", () => ({
  listTokens: listTokensMock,
  createToken: createTokenMock,
  revokeToken: revokeTokenMock,
}));

const identityGroups = { value: ["authentik:eng", "authentik:qa"] };

vi.mock("@/composables/useAuth", () => ({
  useAuth: () => ({
    token: { value: "t" },
    identity: { value: { role: "admin", groups: identityGroups.value } },
  }),
}));

import TokensPage from "./TokensPage.vue";
import { DestructiveConfirm } from "@/components/ui/destructive-confirm";

/**
 * Revocation is irreversible and had no confirmation at all.
 *
 * `revokeToken(tok.id)` fired straight from the bin icon: no dialogue, no
 * scope, no undo, no announcement. A CI pipeline began failing and the only
 * signal connecting the two was the operator's memory of having clicked.
 *
 * The button itself was already correct — its `aria-label` is translated and
 * names the token. What was missing was the confirmation, not the name.
 */
let active: ReturnType<typeof mount> | null = null;

afterEach(() => {
  active?.unmount();
  active = null;
  document.body.innerHTML = "";
});

async function mountPage(tokens?: unknown[]) {
  listTokensMock.mockResolvedValue({
    data: tokens ?? [
      {
        id: "tok-1",
        name: "ci-eu-west",
        role: "user",
        created_at: null,
        expires_at: null,
        groups: [],
      },
    ],
  });
  const wrapper = mount(TokensPage, {
    attachTo: document.body,
    global: { stubs: { RouterLink: { template: "<a><slot /></a>" } } },
  });
  await flushPromises();
  active = wrapper;
  return wrapper;
}

const binFor = (wrapper: ReturnType<typeof mount>, name: string) =>
  wrapper.findAll("button").find((b) => b.attributes("aria-label")?.includes(name))!;

describe("TokensPage revocation", () => {
  beforeEach(() => {
    revokeTokenMock.mockReset().mockResolvedValue({ data: {} });
    createTokenMock.mockReset();
  });

  it("does not revoke on the click itself", async () => {
    const wrapper = await mountPage();
    await binFor(wrapper, "ci-eu-west").trigger("click");
    await flushPromises();

    expect(revokeTokenMock).not.toHaveBeenCalled();
    expect(wrapper.findComponent(DestructiveConfirm).props("open")).toBe(true);
  });

  it("names the token it is about and states what revoking costs", async () => {
    const wrapper = await mountPage();
    await binFor(wrapper, "ci-eu-west").trigger("click");
    await flushPromises();

    const dialog = wrapper.findComponent(DestructiveConfirm);
    expect(dialog.props("scope")).toBe("ci-eu-west");
    expect(dialog.props("count")).toBe(1);
    expect(dialog.props("reversible")).toBeFalsy();

    /* Not `destructive.cannotUndo`. That sentence says "The artifacts and
       their metadata are removed permanently", which is about a delete —
       revoking a token removes no artifact. Shipping it here would state a
       consequence that does not happen while the one that does goes unsaid. */
    const consequence = dialog.props("consequence") as string;
    expect(consequence).toBeTruthy();
    expect(consequence).not.toMatch(/artifacts/i);
  });

  /**
   * `confirmName` on an irreversible action is the case the component's own
   * docstring describes: friction proportional to consequence.
   */
  it("makes the operator type the token's name", async () => {
    const wrapper = await mountPage();
    await binFor(wrapper, "ci-eu-west").trigger("click");
    await flushPromises();

    expect(wrapper.findComponent(DestructiveConfirm).props("confirmName")).toBe("ci-eu-west");
  });

  it("revokes once confirmed", async () => {
    const wrapper = await mountPage();
    await binFor(wrapper, "ci-eu-west").trigger("click");
    await flushPromises();

    wrapper.findComponent(DestructiveConfirm).vm.$emit("confirm");
    await flushPromises();
    expect(revokeTokenMock).toHaveBeenCalledWith({ path: { id: "tok-1" } });
  });

  /**
   * Announced, not only rendered. `Announcer` was mounted on six admin pages
   * and on zero consumer surfaces, so every change a consumer made to their
   * own tokens was silent to a screen reader.
   */
  it("announces the revocation", async () => {
    const wrapper = await mountPage();
    await binFor(wrapper, "ci-eu-west").trigger("click");
    await flushPromises();

    wrapper.findComponent(DestructiveConfirm).vm.$emit("confirm");
    await flushPromises();

    const live = document.querySelector("[aria-live]");
    expect(live?.textContent).toContain("ci-eu-west");
  });

  it("keeps a failed revocation in the dialog rather than closing on it", async () => {
    revokeTokenMock.mockResolvedValue({ error: { message: "not permitted" } });
    const wrapper = await mountPage();
    await binFor(wrapper, "ci-eu-west").trigger("click");
    await flushPromises();

    wrapper.findComponent(DestructiveConfirm).vm.$emit("confirm");
    await flushPromises();

    const dialog = wrapper.findComponent(DestructiveConfirm);
    expect(dialog.props("open")).toBe(true);
    expect(dialog.props("error")).toContain("not permitted");
  });
});

/**
 * RFC 0011-bis §4.4 — a PAT's groups.
 *
 * A token used to resolve to no groups at all, so automation saw `public` and
 * `internal` and nothing its owner was granted through a team. The console is
 * where the snapshot is chosen and, because a snapshot goes stale silently,
 * the only place its owner can see what an existing token still carries.
 */
describe("TokensPage group snapshot", () => {
  beforeEach(() => {
    revokeTokenMock.mockReset().mockResolvedValue({ data: {} });
    createTokenMock.mockReset().mockResolvedValue({
      data: {
        id: "tok-2",
        name: "ci",
        token: "bh_pat_secret",
        role: "user",
        expires_at: null,
        created_at: null,
        groups: ["authentik:eng"],
      },
    });
  });

  /* The create dialog renders through radix's `DialogPortal`, so it lands in
     `document.body` outside the wrapper's own tree — queried through the DOM,
     the way the existing announcer assertion already does. */
  const buttonIn = (root: ParentNode, label: string) =>
    [...root.querySelectorAll("button")].find((b) => b.textContent?.trim() === label)!;

  /* Scoped to the dialog, never to the whole body: the page header carries a
     "Create token" button of its own, and matching that one silently reopens
     the dialog instead of submitting it. */
  const dialog = () => document.querySelector('[role="dialog"]') as ParentNode;

  async function openDialog() {
    const wrapper = await mountPage();
    buttonIn(wrapper.element as ParentNode, "Create token").click();
    await flushPromises();
    return wrapper;
  }

  async function submit(label = "ci") {
    const name = dialog().querySelector("#token-name") as HTMLInputElement;
    name.value = label;
    name.dispatchEvent(new Event("input"));
    await flushPromises();
    buttonIn(dialog(), "Create token").click();
    await flushPromises();
  }

  /** Only what the caller holds: the server refuses anything else with a 403. */
  it("offers exactly the caller's own groups", async () => {
    await openDialog();
    for (const g of identityGroups.value) {
      expect(buttonIn(dialog(), g)).toBeTruthy();
    }
  });

  it("sends no groups when none are picked", async () => {
    await openDialog();
    await submit();

    expect(createTokenMock).toHaveBeenCalledWith(
      expect.objectContaining({ body: expect.objectContaining({ groups: [] }) }),
    );
  });

  it("sends the picked groups", async () => {
    await openDialog();
    buttonIn(dialog(), "authentik:eng").click();
    await flushPromises();
    await submit();

    expect(createTokenMock).toHaveBeenCalledWith(
      expect.objectContaining({
        body: expect.objectContaining({ groups: ["authentik:eng"] }),
      }),
    );
  });

  /**
   * The server's answer, not the form's. A snapshot is capped to what the
   * creator holds, so the two can differ, and what the token *actually*
   * carries is the only one worth showing at the one moment it can be changed.
   */
  it("reveals what the created token carries", async () => {
    const wrapper = await openDialog();
    await submit();

    expect(wrapper.text()).toContain("authentik:eng");
  });

  it("says so when a created token carries nothing", async () => {
    createTokenMock.mockResolvedValue({
      data: {
        id: "tok-3",
        name: "ci",
        token: "bh_pat_secret",
        role: "user",
        expires_at: null,
        created_at: null,
        groups: [],
      },
    });
    const wrapper = await openDialog();
    await submit();

    expect(wrapper.text()).toContain("only public and internal");
  });

  /** Where an owner finds out a token still carries a team they have left. */
  it("lists what an existing token carries", async () => {
    const wrapper = await mountPage([
      {
        id: "tok-1",
        name: "ci-eu-west",
        role: "user",
        created_at: null,
        expires_at: null,
        groups: ["authentik:qa"],
      },
    ]);
    expect(wrapper.text()).toContain("authentik:qa");
  });

  it("marks a groupless token in the listing rather than leaving the cell blank", async () => {
    const wrapper = await mountPage();
    expect(wrapper.text()).toContain("public + internal only");
  });
});
