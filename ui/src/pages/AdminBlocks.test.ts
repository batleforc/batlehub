import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";

const { listUsers, listIps, blockUserMock, unblockUserMock, blockIpMock, unblockIpMock } =
  vi.hoisted(() => ({
    listUsers: vi.fn(),
    listIps: vi.fn(),
    blockUserMock: vi.fn(),
    unblockUserMock: vi.fn(),
    blockIpMock: vi.fn(),
    unblockIpMock: vi.fn(),
  }));

vi.mock("@/client/sdk.gen", () => ({
  listBlockedUsers: listUsers,
  listBlockedIps: listIps,
  blockUser: blockUserMock,
  unblockUser: unblockUserMock,
  blockIp: blockIpMock,
  unblockIp: unblockIpMock,
}));

import AdminBlocks from "./AdminBlocks.vue";
import { DestructiveConfirm } from "@/components/ui/destructive-confirm";

/** Seconds since the epoch, which is what `BlockedIpDto` carries. */
const epoch = (offsetMs: number) => Math.floor((Date.now() + offsetMs) / 1000);

async function mountPage() {
  const wrapper = mount(AdminBlocks, {
    // `Dialog` teleports, so the tree has to be in the document for the
    // dialog's own content to be reachable at all.
    attachTo: document.body,
    global: { stubs: { RouterLink: { template: "<a><slot /></a>" }, SectionTabs: true } },
  });
  await flushPromises();
  return wrapper;
}

describe("AdminBlocks", () => {
  beforeEach(() => {
    listUsers.mockReset().mockResolvedValue({
      data: [
        {
          user_id: "svc-ci-runner-eu-west-1",
          reason: "Leaked token",
          blocked_at: "2026-08-10T14:22:00Z",
          blocked_by: "alice",
        },
      ],
    });
    listIps.mockReset().mockResolvedValue({
      data: [
        {
          ip: "203.0.113.42",
          reason: "Sustained 429s against the npm upstream",
          blocked_at: epoch(-3_600_000),
          unblock_at: epoch(3_600_000),
        },
      ],
    });
    blockUserMock.mockReset().mockResolvedValue({ data: {} });
    unblockUserMock.mockReset().mockResolvedValue({ data: {} });
    blockIpMock.mockReset().mockResolvedValue({ data: {} });
    unblockIpMock.mockReset().mockResolvedValue({ data: {} });
  });

  /**
   * The point of the merge (RFC 0004 Phase 5): an operator arrives with a
   * symptom — "the EU CI runner started failing" — not with a mechanism, and
   * previously had to visit two routes because neither mentioned the other.
   */
  it("shows both mechanisms in one table, labelled by kind", async () => {
    const wrapper = await mountPage();
    const rows = wrapper.findAll("tbody tr");
    expect(rows).toHaveLength(2);
    expect(wrapper.text()).toContain("svc-ci-runner-eu-west-1");
    expect(wrapper.text()).toContain("203.0.113.42");
    expect(wrapper.text()).toMatch(/Account/);
    expect(wrapper.text()).toMatch(/Address/);
  });

  it("renders a block with no reason without inventing one", async () => {
    listUsers.mockResolvedValue({
      data: [{ user_id: "carol", reason: null, blocked_at: null, blocked_by: "admin" }],
    });
    listIps.mockResolvedValue({ data: [] });
    const wrapper = await mountPage();
    expect(wrapper.text()).toContain("carol");
    expect(wrapper.text()).toContain("—");
  });

  /**
   * An expired IP block is history, not a block. Both source pages kept it in
   * the list at reduced opacity with a working "Unblock" button — an action
   * offered for something already unblocked.
   */
  it("marks an expired address block and offers no verb for it", async () => {
    listUsers.mockResolvedValue({ data: [] });
    listIps.mockResolvedValue({
      data: [
        { ip: "198.51.100.7", reason: "auto", blocked_at: epoch(-7_200_000), unblock_at: epoch(-60_000) },
      ],
    });
    const wrapper = await mountPage();
    expect(wrapper.text()).toMatch(/expired/i);
    const row = wrapper.get("tbody tr");
    expect(row.findAll("button")).toHaveLength(0);
  });

  /**
   * The middleware writes `"auto"` when an address crosses the violation
   * threshold. Those are the entries an operator did *not* put there, and the
   * bare word told them nothing.
   */
  it("explains a machine-created block rather than printing 'auto'", async () => {
    listUsers.mockResolvedValue({ data: [] });
    listIps.mockResolvedValue({
      data: [{ ip: "198.51.100.9", reason: "auto", blocked_at: epoch(-600_000), unblock_at: epoch(600_000) }],
    });
    const wrapper = await mountPage();
    expect(wrapper.text()).toMatch(/automatic/i);
  });

  it("filters across both kinds by subject and by reason", async () => {
    const wrapper = await mountPage();
    await wrapper.get("input").setValue("429s");
    await flushPromises();
    const rows = wrapper.findAll("tbody tr");
    expect(rows).toHaveLength(1);
    expect(wrapper.text()).toContain("203.0.113.42");
  });

  /**
   * `DestructiveConfirm` names IP blocks in its own doc comment as one of the
   * four reasons it exists, and neither source page used it.
   */
  it("routes lifting a block through the destructive contract", async () => {
    const wrapper = await mountPage();
    const lift = wrapper.findAll("button").find((b) => /lift/i.test(b.text()))!;
    await lift.trigger("click");
    await flushPromises();

    const dialog = wrapper.findComponent(DestructiveConfirm);
    expect(dialog.props("open")).toBe(true);
    expect(dialog.props("scope")).toBeTruthy();

    dialog.vm.$emit("confirm");
    await flushPromises();
    expect(unblockUserMock.mock.calls.length + unblockIpMock.mock.calls.length).toBe(1);
  });

  it("surfaces a failed block instead of closing silently", async () => {
    blockUserMock.mockResolvedValue({ error: { message: "not permitted" } });
    const wrapper = await mountPage();
    const open = wrapper.findAll("button").find((b) => /block account/i.test(b.text()))!;
    await open.trigger("click");
    await flushPromises();

    // `Dialog` teleports its content out of the wrapper, so the form lives on
    // the document rather than inside the mounted tree.
    const subject = document.querySelector<HTMLInputElement>("#block-subject")!;
    subject.value = "mallory";
    subject.dispatchEvent(new Event("input"));
    await flushPromises();

    document.querySelector<HTMLButtonElement>('[data-testid="block-submit"]')!.click();
    await flushPromises();
    expect(document.body.textContent).toContain("not permitted");
  });

  it("says nobody is blocked when nobody is", async () => {
    listUsers.mockResolvedValue({ data: [] });
    listIps.mockResolvedValue({ data: [] });
    const wrapper = await mountPage();
    expect(wrapper.text()).toMatch(/nobody is blocked/i);
  });
});
