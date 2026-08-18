import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";

/**
 * The **Fetch this version** button (RFC 0007-bis §4.4).
 *
 * The page had no test file. This one is scoped to the button rather than to the
 * page: the endpoint's own suite (`crates/web/tests/explore_fetch.rs`) covers
 * what the fetch *does*, and what the console owes the reader is narrower — that
 * the button appears only where it can work, that a refusal is shown verbatim
 * rather than swallowed, and that the row is refreshed rather than left claiming
 * the version is still upstream-only.
 */

const { explorePackageDetailMock, exploreFetchVersionMock, listRegistriesMock, packageDetailMock } =
  vi.hoisted(() => ({
    explorePackageDetailMock: vi.fn(),
    exploreFetchVersionMock: vi.fn(),
    listRegistriesMock: vi.fn(),
    packageDetailMock: vi.fn(),
  }));

vi.mock("@/client/sdk.gen", () => ({
  explorePackageDetail: explorePackageDetailMock,
  exploreFetchVersion: exploreFetchVersionMock,
  listRegistries: listRegistriesMock,
  packageDetail: packageDetailMock,
}));

vi.mock("vue-router", () => ({
  useRoute: () => ({ params: { registry: "npm1", name: "express" } }),
  useRouter: () => ({ push: vi.fn() }),
  RouterLink: { template: "<a><slot /></a>" },
}));

vi.mock("@/composables/useAuth", () => ({
  useAuth: () => ({ token: { value: "t" }, isAdmin: { value: false } }),
}));

vi.mock("@/composables/useAuthFetch", () => ({
  useAuthFetch: () => ({ authFetch: vi.fn() }),
}));

import PackageDetailPage from "./PackageDetailPage.vue";

function version(over: Record<string, unknown> = {}) {
  return {
    version: "4.18.2",
    source: "upstream",
    firewall: { status: "allowed" },
    download_count: null,
    last_accessed: null,
    published_at: null,
    is_prerelease: false,
    vulnerabilities: [],
    readme: "unknown",
    vulnerabilities_scanned: false,
    ...over,
  };
}

function detail(over: Record<string, unknown> = {}) {
  return {
    data: {
      registry: "npm1",
      name: "express",
      gate: { registry_accessible: true, beta_member: false },
      versions: [version()],
      upstream_unavailable: false,
      upstream: { attempted: true, freshness: "cached", truncated: false, error: null },
      fetch: { offered: true, reason: null },
      ...over,
    },
  };
}

async function mountPage() {
  const wrapper = mount(PackageDetailPage, {
    global: {
      stubs: {
        RouterLink: { template: "<a><slot /></a>" },
        ReadmePanel: true,
        UpstreamNotice: true,
        PackageVersionsTable: true,
        PackageBetaChannel: true,
        PackageVisibility: true,
        PackageEventsTable: true,
      },
    },
  });
  await flushPromises();
  return wrapper;
}

const fetchButton = (w: Awaited<ReturnType<typeof mountPage>>) =>
  w.findAll("button").find((b) => b.text().includes("Fetch this version"));

describe("PackageDetailPage fetch button", () => {
  beforeEach(() => {
    explorePackageDetailMock.mockReset().mockResolvedValue(detail());
    exploreFetchVersionMock.mockReset();
    listRegistriesMock.mockReset().mockResolvedValue({ data: [{ name: "npm1", type: "npm" }] });
    packageDetailMock.mockReset().mockResolvedValue({ data: null });
  });

  /**
   * The door beside the wall. RFC 0007 made the row honest about not holding the
   * version; this is what a reader can do about it.
   */
  it("is offered on an upstream-only row", async () => {
    const wrapper = await mountPage();
    expect(fetchButton(wrapper)).toBeDefined();
  });

  /** A version already held needs no fetching, and offering one would be noise. */
  it("is not offered on a row this instance already holds", async () => {
    explorePackageDetailMock.mockResolvedValue(
      detail({ versions: [version({ source: "proxied" })] }),
    );
    const wrapper = await mountPage();
    expect(fetchButton(wrapper)).toBeUndefined();
  });

  /**
   * Not a disabled control with no explanation. Where "fetch this version" has
   * no single meaning, the kind's own reason is shown — the same string the
   * endpoint and the published support table use (RFC 0007-bis §4.4).
   */
  it("shows the kind's reason instead of a button it cannot honour", async () => {
    explorePackageDetailMock.mockResolvedValue(
      detail({
        fetch: { offered: false, reason: "a Maven version is a set of files" },
      }),
    );
    const wrapper = await mountPage();

    expect(fetchButton(wrapper)).toBeUndefined();
    expect(wrapper.text()).toContain("a Maven version is a set of files");
  });

  /**
   * The operator turned it off. Nothing is offered and nothing is explained:
   * describing an operator's own configuration back to them on a package page is
   * noise, not information.
   */
  it("says nothing at all when the operator turned it off", async () => {
    explorePackageDetailMock.mockResolvedValue(detail({ fetch: { offered: false, reason: null } }));
    const wrapper = await mountPage();

    expect(fetchButton(wrapper)).toBeUndefined();
    expect(wrapper.text()).not.toContain("Not fetchable");
  });

  it("fetches the row's own version, and refreshes so the row stops saying upstream", async () => {
    exploreFetchVersionMock.mockResolvedValue({
      data: { fetched: true, size_bytes: 4096, duration_ms: 12 },
    });
    const wrapper = await mountPage();
    const before = explorePackageDetailMock.mock.calls.length;

    await fetchButton(wrapper)!.trigger("click");
    await flushPromises();

    expect(exploreFetchVersionMock).toHaveBeenCalledWith({
      path: { registry: "npm1", name: "express", version: "4.18.2" },
    });
    // Refreshed rather than leaving the reader to reload and wonder.
    expect(explorePackageDetailMock.mock.calls.length).toBeGreaterThan(before);
    expect(wrapper.text()).toContain("Fetched");
  });

  /**
   * The rule's own reason, verbatim — the same string the download would have
   * given, so an operator can take it to the RBAC simulator and get the same
   * verdict explained (RFC 0007-bis §4.4).
   */
  it("shows a refusal's reason rather than a generic failure", async () => {
    exploreFetchVersionMock.mockResolvedValue({
      error: { code: "fetch.denied", message: "blocked by release-age gate (3 days remaining)" },
    });
    const wrapper = await mountPage();

    await fetchButton(wrapper)!.trigger("click");
    await flushPromises();

    expect(wrapper.text()).toContain("release-age gate");
  });

  /** A version that arrived between the page load and the press. */
  it("reports an already-held conflict in its own words", async () => {
    exploreFetchVersionMock.mockResolvedValue({
      error: { code: "fetch.already-held", message: "this instance already holds express 4.18.2" },
    });
    const wrapper = await mountPage();

    await fetchButton(wrapper)!.trigger("click");
    await flushPromises();

    expect(wrapper.text()).toContain("Already held");
  });
});
