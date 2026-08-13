import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";

const { registryHealthMock, listNamespacesMock, claimMock, releaseMock, listSubjectsMock } =
  vi.hoisted(() => ({
    registryHealthMock: vi.fn(),
    listNamespacesMock: vi.fn(),
    claimMock: vi.fn(),
    releaseMock: vi.fn(),
    listSubjectsMock: vi.fn(),
  }));
vi.mock("@/client/sdk.gen", () => ({
  registryHealth: registryHealthMock,
  listNamespaces: listNamespacesMock,
  claimNamespace: claimMock,
  releaseNamespace: releaseMock,
  listSubjects: listSubjectsMock,
  explorePackages: vi.fn(),
  explorePackageDetail: vi.fn(),
}));

import AdminTeamNamespaces from "./AdminTeamNamespaces.vue";

const ns = (over: Record<string, unknown> = {}) => ({
  registry: "cargo",
  prefix: "frontend",
  group_id: "team-fe",
  claimed_by: "oidc:alice",
  package_count: 42,
  ...over,
});

/** Never resolves — the state the empty message must not be shown in. */
const pending = () => new Promise(() => {});

async function mountPage() {
  const wrapper = mount(AdminTeamNamespaces, {
    global: { stubs: { SectionTabs: true, RouterLink: true } },
  });
  await flushPromises();
  return wrapper;
}

/**
 * The page's question: "which prefixes are claimed, by whom, and how much is
 * in them".
 */
describe("AdminTeamNamespaces", () => {
  beforeEach(() => {
    registryHealthMock.mockReset().mockResolvedValue({
      data: [{ registry: "cargo", registry_type: "cargo" }],
    });
    listNamespacesMock.mockReset().mockResolvedValue({ data: [ns()] });
    claimMock.mockReset().mockResolvedValue({ data: {} });
    releaseMock.mockReset().mockResolvedValue({ data: {} });
    listSubjectsMock.mockReset().mockResolvedValue({ data: { items: [], truncated: false } });
  });

  /**
   * A6: `count_packages_in_namespace` had been on the port since it was written
   * with the *delete confirmation* as its only caller, so this list could not
   * tell a namespace holding four hundred packages from an abandoned one.
   */
  it("states how many packages each claim holds", async () => {
    listNamespacesMock.mockResolvedValue({
      data: [
        ns({ prefix: "frontend", package_count: 42 }),
        ns({ prefix: "dead", package_count: 0 }),
      ],
    });
    const wrapper = await mountPage();
    const counts = wrapper.findAll("tbody tr").map((row) => row.findAll("td").at(-2)!.text());
    // Zero is the answer that matters here, so it is rendered rather than blanked.
    expect(counts).toEqual(["42", "0"]);
  });

  /**
   * §4.3: "none for this registry" must never render while loading.
   *
   * It did, so an operator read the empty state, opened the dialog and claimed
   * a prefix that already existed. Empty must mean empty, not "we have not
   * looked yet".
   */
  it("does not claim the registry is empty while it is still loading", async () => {
    listNamespacesMock.mockReturnValue(pending());
    const wrapper = await mountPage();
    expect(wrapper.text()).not.toMatch(/no namespaces|none for this registry/i);
  });

  it("says so when a registry genuinely has no claims", async () => {
    listNamespacesMock.mockResolvedValue({ data: [] });
    const wrapper = await mountPage();
    expect(wrapper.text()).toMatch(/no namespace/i);
  });

  it("surfaces a load error rather than an empty list", async () => {
    listNamespacesMock.mockResolvedValue({ error: { message: "cannot reach db" } });
    const wrapper = await mountPage();
    expect(wrapper.text()).toContain("cannot reach db");
  });
});
