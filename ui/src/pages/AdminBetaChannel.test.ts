import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";

const { registryHealthMock, listMembersMock, addMock, removeMock, listSubjectsMock } = vi.hoisted(
  () => ({
    registryHealthMock: vi.fn(),
    listMembersMock: vi.fn(),
    addMock: vi.fn(),
    removeMock: vi.fn(),
    listSubjectsMock: vi.fn(),
  }),
);
vi.mock("@/client/sdk.gen", () => ({
  registryHealth: registryHealthMock,
  listBetaMembers: listMembersMock,
  addBetaMember: addMock,
  removeBetaMember: removeMock,
  listSubjects: listSubjectsMock,
  explorePackages: vi.fn(),
  explorePackageDetail: vi.fn(),
}));

import AdminBetaChannel from "./AdminBetaChannel.vue";

const member = (over: Record<string, unknown> = {}) => ({
  registry: "npm",
  principal_type: "user",
  principal_id: "oidc:alice",
  granted_by: "admin",
  granted_at: "2026-08-01T00:00:00Z",
  ...over,
});

const pending = () => new Promise(() => {});

async function mountPage() {
  const wrapper = mount(AdminBetaChannel, {
    global: { stubs: { SectionTabs: true, RouterLink: true } },
  });
  await flushPromises();
  return wrapper;
}

/** The page's question: "who is on the beta channel for this registry". */
describe("AdminBetaChannel", () => {
  beforeEach(() => {
    registryHealthMock.mockReset().mockResolvedValue({
      data: [{ registry: "npm", registry_type: "npm" }],
    });
    listMembersMock.mockReset().mockResolvedValue({ data: [member()] });
    addMock.mockReset().mockResolvedValue({ data: {} });
    removeMock.mockReset().mockResolvedValue({ data: {} });
    listSubjectsMock.mockReset().mockResolvedValue({ data: { items: [], truncated: false } });
  });

  it("lists the members of the selected registry", async () => {
    const wrapper = await mountPage();
    expect(wrapper.text()).toContain("oidc:alice");
  });

  /**
   * §4.3: "none for this registry" must never render while loading — the same
   * defect as `AdminTeamNamespaces`, on the page beside it. An operator who
   * reads an empty state that means "not yet fetched" grants access twice.
   */
  it("does not claim the channel is empty while it is still loading", async () => {
    listMembersMock.mockReturnValue(pending());
    const wrapper = await mountPage();
    expect(wrapper.text()).not.toMatch(/no beta channel members/i);
  });

  it("says so when the channel genuinely has no members", async () => {
    listMembersMock.mockResolvedValue({ data: [] });
    const wrapper = await mountPage();
    expect(wrapper.text()).toMatch(/no beta channel members/i);
  });

  it("surfaces a load error rather than an empty channel", async () => {
    listMembersMock.mockResolvedValue({ error: { message: "cannot reach db" } });
    const wrapper = await mountPage();
    expect(wrapper.text()).toContain("cannot reach db");
  });
});
