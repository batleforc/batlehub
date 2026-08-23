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

vi.mock("@/composables/useAuth", () => ({
  useAuth: () => ({ token: { value: "t" }, identity: { value: { role: "admin" } } }),
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

async function mountPage() {
  listTokensMock.mockResolvedValue({
    data: [{ id: "tok-1", name: "ci-eu-west", role: "user", created_at: null, expires_at: null }],
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
