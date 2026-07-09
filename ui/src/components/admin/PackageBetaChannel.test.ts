import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";

const { listBetaMembersMock } = vi.hoisted(() => ({ listBetaMembersMock: vi.fn() }));

vi.mock("@/client/sdk.gen", () => ({ listBetaMembers: listBetaMembersMock }));

import PackageBetaChannel from "./PackageBetaChannel.vue";

async function mountComp() {
  const wrapper = mount(PackageBetaChannel, { props: { registry: "npm" } });
  await flushPromises();
  return wrapper;
}

describe("PackageBetaChannel", () => {
  beforeEach(() => {
    listBetaMembersMock.mockReset().mockResolvedValue({
      data: [
        { principal_type: "user", principal_id: "alice", granted_by: "admin" },
        { principal_type: "group", principal_id: "qa-team", granted_by: null },
      ],
    });
  });

  it("starts collapsed", async () => {
    const wrapper = await mountComp();
    expect(wrapper.find("table").exists()).toBe(false);
  });

  it("shows the member count badge once loaded", async () => {
    const wrapper = await mountComp();
    expect(wrapper.text()).toContain("2 members");
  });

  it("expands to show the member table on click", async () => {
    const wrapper = await mountComp();
    await wrapper.find("button").trigger("click");
    expect(wrapper.find("table").exists()).toBe(true);
    expect(wrapper.findAll("tbody tr")).toHaveLength(2);
    expect(wrapper.text()).toContain("alice");
    expect(wrapper.text()).toContain("qa-team");
    expect(wrapper.text()).toContain("—"); // null granted_by
  });

  it("shows the empty state when there are no members", async () => {
    listBetaMembersMock.mockResolvedValueOnce({ data: [] });
    const wrapper = await mountComp();
    await wrapper.find("button").trigger("click");
    expect(wrapper.text()).toContain("No beta channel members");
  });

  it("reloads members when Refresh is clicked", async () => {
    const wrapper = await mountComp();
    await wrapper.find("button").trigger("click");
    const refresh = wrapper.findAll("button").find((b) => b.text().includes("Refresh"))!;
    await refresh.trigger("click");
    await flushPromises();
    expect(listBetaMembersMock).toHaveBeenCalledTimes(2);
  });
});
