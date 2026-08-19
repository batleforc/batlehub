import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
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

const {
  explorePackageDetailMock,
  exploreFetchVersionMock,
  listRegistriesMock,
  packageDetailMock,
  pushMock,
  backMock,
  replaceMock,
  routeState,
} = vi.hoisted(() => ({
  explorePackageDetailMock: vi.fn(),
  exploreFetchVersionMock: vi.fn(),
  listRegistriesMock: vi.fn(),
  packageDetailMock: vi.fn(),
  pushMock: vi.fn(),
  backMock: vi.fn(),
  replaceMock: vi.fn(),
  routeState: {
    path: "/packages/npm1/express",
    params: { registry: "npm1", name: "express" },
    query: {} as Record<string, string>,
  },
}));

vi.mock("@/client/sdk.gen", () => ({
  explorePackageDetail: explorePackageDetailMock,
  exploreFetchVersion: exploreFetchVersionMock,
  listRegistries: listRegistriesMock,
  packageDetail: packageDetailMock,
}));

/**
 * A route with a `path` and a `query`, because the page now reads and writes
 * both: the selected version travels in the query, so a mock without one cannot
 * tell whether a link to a version opens on that version.
 */
vi.mock("vue-router", () => ({
  useRoute: () => routeState,
  useRouter: () => ({ push: pushMock, back: backMock, replace: replaceMock }),
  RouterLink: { template: "<a><slot /></a>" },
}));

/**
 * Signed in, not an admin, by default; individual tests flip a field before
 * mounting.
 *
 * **Real refs.** This mock returned plain `{ value }` objects, and a plain object
 * is truthy in a template however its `.value` reads — so `v-if="isAdmin"` was
 * true for every test in this file and the suite had been rendering the whole
 * Administration section for a viewer it had declared a non-admin. Anything the
 * page decides *in the template* from an auth flag was therefore untestable, and
 * the first test to need one found out.
 */
const { authState } = vi.hoisted(() => ({
  authState: { token: "t", isAdmin: false, isAuthenticated: true },
}));
vi.mock("@/composables/useAuth", async () => {
  const { ref } = await import("vue");
  return {
    useAuth: () => ({
      token: ref(authState.token),
      isAdmin: ref(authState.isAdmin),
      isAuthenticated: ref(authState.isAuthenticated),
    }),
  };
});

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

/**
 * The endpoint's own rules, in the double.
 *
 * The filter, the pager and the pre-release toggle are the server's since the
 * answer became one page (RFC 0013 §4.3). A stub that returned a fixed list
 * whatever was asked of it would let this suite go on passing while the page
 * sent nonsense — the assertions would be about a fiction the double maintains.
 * So it filters, pages and counts the way `explore/detail.rs` does, and the
 * tests below assert what the page *asks for* and what it does with the answer.
 *
 * Kept deliberately literal, in the same order as the handler: pre-releases
 * (with the pinned version surviving), then `q` (which it does not), then the
 * page — whose default is the page holding the pin.
 */
function serve(over: Record<string, unknown> = {}) {
  const base = detail(over).data as Record<string, unknown>;
  const all = base.versions as Array<{ version: string; is_prerelease: boolean; source: string }>;

  const defaultVersion = (() => {
    const held = (v: { source: string }) => v.source !== "upstream";
    const stable = all.filter((v) => !v.is_prerelease);
    const pick = stable.find(held) ?? stable[0] ?? all.find(held) ?? all[0];
    return pick?.version ?? null;
  })();

  return (opts: { query?: Record<string, string | number> } = {}) => {
    const query = opts.query ?? {};
    const pin = query.version === undefined ? null : String(query.version);
    const unfilteredTotal = all.length;
    const prereleaseTotal = all.filter((v) => v.is_prerelease).length;

    let rows = all;
    if (query.prereleases === "hide") {
      // The pinned version survives, and so does the default one — a package
      // that has only ever cut pre-releases would otherwise answer with an
      // empty table.
      rows = rows.filter(
        (v) => !v.is_prerelease || v.version === pin || v.version === defaultVersion,
      );
    }
    const hiddenPrereleases = unfilteredTotal - rows.length;

    const needle = String(query.q ?? "")
      .trim()
      .toLowerCase();
    if (needle) rows = rows.filter((v) => v.version.toLowerCase().includes(needle));
    const total = rows.length;

    const perPage = Math.max(1, Math.min(Number(query.per_page ?? 100), 100));
    const lastPage = Math.max(0, Math.ceil(total / perPage) - 1);
    const asked =
      query.page !== undefined
        ? Number(query.page)
        : pin === null
          ? 0
          : Math.max(0, Math.floor(rows.findIndex((v) => v.version === pin) / perPage));
    const page = Math.min(asked, lastPage);

    return Promise.resolve({
      data: {
        ...base,
        versions: rows.slice(page * perPage, page * perPage + perPage),
        versions_page: {
          page,
          per_page: perPage,
          total,
          unfiltered_total: unfilteredTotal,
          prerelease_total: prereleaseTotal,
          hidden_prereleases: hiddenPrereleases,
        },
        default_version: defaultVersion,
        selected_version: pin !== null && all.some((v) => v.version === pin) ? pin : null,
      },
    });
  };
}

/**
 * A response with the envelope written by hand.
 *
 * `serve` derives it, which is right for the tests about the controls and wrong
 * for the tests about *delegation*: those have to be able to state what the
 * server said — including something no rule would have produced — and watch the
 * page obey it.
 */
function respond(versions: unknown[], envelope: Record<string, unknown> = {}) {
  const base = detail({ versions }).data;
  return {
    data: {
      ...base,
      versions_page: {
        page: 0,
        per_page: 25,
        total: versions.length,
        unfiltered_total: versions.length,
        prerelease_total: 0,
        hidden_prereleases: 0,
      },
      default_version: null,
      selected_version: null,
      ...envelope,
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

/** Type into the version filter and let the 300 ms debounce elapse — it costs a
    request now, so a keystroke is not an answer until the timer fires. */
async function typeFilter(w: Awaited<ReturnType<typeof mountPage>>, value: string) {
  await w.find("input").setValue(value);
  await new Promise((resolve) => setTimeout(resolve, 350));
  await flushPromises();
}

const fetchButton = (w: Awaited<ReturnType<typeof mountPage>>) =>
  w.findAll("button").find((b) => b.text().includes("Fetch this version"));

describe("PackageDetailPage fetch button", () => {
  beforeEach(() => {
    explorePackageDetailMock.mockReset().mockImplementation(serve());
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
    explorePackageDetailMock.mockImplementation(
      serve({ versions: [version({ source: "proxied" })] }),
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
    explorePackageDetailMock.mockImplementation(
      serve({
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
    explorePackageDetailMock.mockImplementation(serve({ fetch: { offered: false, reason: null } }));
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

/**
 * Which version the page opens on.
 *
 * The endpoint sorts stable-before-pre-release, newest-first, and upstream-only
 * rows sit in that list beside the ones this instance holds. Taking `versions[0]`
 * therefore opened any package the world had moved past on a row we do not have —
 * with the README panel following that selection.
 *
 * The rows are asserted through the selected row's marker rather than through the
 * component's internals: what the reader gets is a highlighted row and a README
 * for that version, and neither should depend on how the page stores it.
 */
describe("PackageDetailPage default selection", () => {
  beforeEach(() => {
    exploreFetchVersionMock.mockReset();
    listRegistriesMock.mockReset().mockResolvedValue({ data: [] });
    packageDetailMock.mockReset().mockResolvedValue({ data: undefined });
  });

  /**
   * The row the page marks as selected, read through `aria-current` rather than
   * through a class.
   *
   * The marker itself is a design decision that has already changed once — it
   * was a `bg-muted/40` fill, which DESIGN.md's Undependable Fill Rule rules out
   * on this ground, and is now a lit edge plus ink weight. `aria-current` is the
   * part that is a contract: it is what a screen reader is told, and it should
   * not have to be rewritten the next time the mark is redrawn.
   */
  const selectedRow = (w: Awaited<ReturnType<typeof mountPage>>) =>
    w.findAll("tbody tr").find((r) => r.attributes("aria-current") === "true");

  it("marks the version the server names as the default", async () => {
    // A default a client rule would not have picked: 4.0.4 is first, and the
    // page must still mark 1.0.0. The rule itself — newest stable *held* —
    // belongs to `explore/detail.rs`, which is the only side that sees every
    // version, and is tested there.
    explorePackageDetailMock.mockResolvedValue(
      respond([version({ version: "4.0.4" }), version({ version: "1.0.0", source: "proxied" })], {
        default_version: "1.0.0",
      }),
    );

    const wrapper = await mountPage();

    expect(selectedRow(wrapper)!.text()).toContain("1.0.0");
    expect(wrapper.findComponent({ name: "ReadmePanel" }).props("version")).toBe("1.0.0");
  });

  /**
   * A version the URL named that this package does not have — a typo, or one
   * yanked since the link was sent — comes back as `selected_version: null`.
   * The page cannot tell that from "on another page" by itself, which is why
   * the endpoint answers it.
   */
  it("falls back to the default when the server does not echo the version asked for", async () => {
    routeState.query = { version: "9.9.9" };
    explorePackageDetailMock.mockResolvedValue(
      respond([version({ version: "4.0.4" }), version({ version: "1.0.0", source: "proxied" })], {
        default_version: "1.0.0",
        selected_version: null,
      }),
    );

    const wrapper = await mountPage();

    expect(selectedRow(wrapper)!.text()).toContain("1.0.0");
    routeState.query = {};
  });

  /** Nothing to select, and nothing marked — not row one by default. */
  it("marks no row for a package with no versions", async () => {
    explorePackageDetailMock.mockResolvedValue(respond([], { default_version: null }));

    const wrapper = await mountPage();

    expect(selectedRow(wrapper)).toBeUndefined();
  });
});

/**
 * Getting back to the list you came from.
 *
 * The button pushed `/packages?registry=…`, which lost the search, the scope,
 * the sort, the page and the scroll offset — and the registry too, since the
 * catalog did not read its own query at the time. The catalog now keeps its
 * state in the URL, so the previous history entry *is* the search, and returning
 * to it is `router.back()` rather than a location rebuilt from one field.
 */
describe("PackageDetailPage back link", () => {
  const backButton = (w: Awaited<ReturnType<typeof mountPage>>) =>
    w.findAll("button").find((b) => b.text().includes("Back to catalog"))!;

  beforeEach(() => {
    pushMock.mockReset();
    backMock.mockReset();
    explorePackageDetailMock.mockReset().mockImplementation(serve());
    listRegistriesMock.mockReset().mockResolvedValue({ data: [] });
    packageDetailMock.mockReset().mockResolvedValue({ data: undefined });
    window.history.replaceState({}, "");
  });

  it("returns to the search the reader came from", async () => {
    // What Vue Router records when the catalog pushed this page.
    window.history.replaceState({ back: "/packages?registry=npm&q=chalk&sort=name" }, "");
    const wrapper = await mountPage();

    await backButton(wrapper).trigger("click");

    expect(backMock).toHaveBeenCalledOnce();
    expect(pushMock).not.toHaveBeenCalled();
  });

  it("pushes the catalog when there is no history to go back to", async () => {
    // A pasted link, a refresh, a new tab.
    const wrapper = await mountPage();

    await backButton(wrapper).trigger("click");

    expect(backMock).not.toHaveBeenCalled();
    expect(pushMock).toHaveBeenCalledWith({ path: "/packages", query: { registry: "npm1" } });
  });

  /**
   * Another package is not "back". Both paths start with `/packages`, so a
   * prefix test would walk the reader sideways through their own history
   * instead of returning them to the list.
   */
  it("does not treat a neighbouring package page as the catalog", async () => {
    window.history.replaceState({ back: "/packages/npm/left-pad" }, "");
    const wrapper = await mountPage();

    await backButton(wrapper).trigger("click");

    expect(backMock).not.toHaveBeenCalled();
    expect(pushMock).toHaveBeenCalledWith({ path: "/packages", query: { registry: "npm1" } });
  });
});

/**
 * The selected version is a destination.
 *
 * "Look at 4.0.2 of this package" is a thing one person sends another, and it
 * was unsendable: the selection was component state, so every link landed on
 * whatever the page chose for itself, and the page's own Refresh discarded what
 * the reader had opened.
 *
 * The rule matches the catalog's, which is the point of writing it twice: the
 * query carries the selection only when it is not the default, and a value the
 * package does not have falls back rather than selecting nothing.
 */
describe("PackageDetailPage version in the url", () => {
  const held = [
    version({ version: "4.0.4" }),
    version({ version: "4.0.2" }),
    version({ version: "2.1.0", source: "proxied" }),
  ];

  const selectedRow = (w: Awaited<ReturnType<typeof mountPage>>) =>
    w.findAll("tbody tr").find((r) => r.attributes("aria-current") === "true");

  beforeEach(() => {
    routeState.query = {};
    replaceMock.mockReset().mockImplementation((loc: { query?: Record<string, string> }) => {
      routeState.query = loc.query ?? {};
    });
    explorePackageDetailMock.mockReset().mockImplementation(serve({ versions: held }));
    listRegistriesMock.mockReset().mockResolvedValue({ data: [] });
    packageDetailMock.mockReset().mockResolvedValue({ data: undefined });
  });

  afterEach(() => {
    routeState.query = {};
  });

  it("opens on the version the link names", async () => {
    routeState.query = { version: "4.0.2" };

    const wrapper = await mountPage();

    expect(selectedRow(wrapper)!.text()).toContain("4.0.2");
    expect(wrapper.findComponent({ name: "ReadmePanel" }).props("version")).toBe("4.0.2");
  });

  it("writes a chosen version into the url", async () => {
    const wrapper = await mountPage();

    await wrapper.findAll("tbody tr")[0].trigger("click"); // 4.0.4, not the default

    expect(replaceMock).toHaveBeenLastCalledWith({
      path: "/packages/npm1/express",
      query: { version: "4.0.4" },
    });
  });

  /** One state, one URL — the same reason the catalog keeps its defaults out. */
  it("keeps the default selection out of the url", async () => {
    const wrapper = await mountPage();
    await wrapper.findAll("tbody tr")[0].trigger("click"); // away from the default
    replaceMock.mockClear();

    await wrapper.findAll("tbody tr")[2].trigger("click"); // 2.1.0, the held default

    expect(replaceMock).toHaveBeenLastCalledWith({
      path: "/packages/npm1/express",
      query: {},
    });
  });

  /** A typo, or a version yanked since the link was sent. */
  it("falls back to the default when the url names a version this package lacks", async () => {
    routeState.query = { version: "9.9.9" };

    const wrapper = await mountPage();

    expect(selectedRow(wrapper)!.text()).toContain("2.1.0");
  });

  /** Refresh re-reads the list; it must not throw away what the reader opened. */
  it("keeps the reader's version across a refresh", async () => {
    routeState.query = { version: "4.0.2" };
    const wrapper = await mountPage();

    await wrapper
      .findAll("button")
      .find((b) => b.text().includes("Refresh"))!
      .trigger("click");
    await flushPromises();

    expect(selectedRow(wrapper)!.text()).toContain("4.0.2");
  });

  it("leaves the query of another page alone", async () => {
    const wrapper = await mountPage();
    routeState.path = "/packages";

    await wrapper.findAll("tbody tr")[0].trigger("click");

    expect(replaceMock).not.toHaveBeenCalled();
    routeState.path = "/packages/npm1/express";
  });
});

/**
 * A pull is an authenticated act.
 *
 * The endpoint refuses a session-less caller with `401 fetch.unauthenticated`
 * and stops advertising the offer (`explore_fetch.rs`), so the console must not
 * draw the button — and must say why rather than leaving a blank where a control
 * was, which is the "disabled control with no explanation" RFC 0007-bis §4.4
 * refuses in the other direction.
 */
describe("PackageDetailPage fetch and the signed-out reader", () => {
  beforeEach(() => {
    routeState.query = {};
    replaceMock.mockReset();
    exploreFetchVersionMock.mockReset();
    listRegistriesMock.mockReset().mockResolvedValue({ data: [] });
    packageDetailMock.mockReset().mockResolvedValue({ data: undefined });
    authState.isAuthenticated = true;
  });

  afterEach(() => {
    authState.isAuthenticated = true;
  });

  it("offers no button and says a session is needed", async () => {
    authState.isAuthenticated = false;
    // What the server sends a session-less reader: no offer, and no reason,
    // because whether there is a session is the half the page knows.
    explorePackageDetailMock.mockImplementation(serve({ fetch: { offered: false, reason: null } }));

    const wrapper = await mountPage();

    expect(fetchButton(wrapper)).toBeUndefined();
    expect(wrapper.text()).toContain("Sign in to fetch this version");
  });

  it("keeps offering it to a signed-in reader", async () => {
    explorePackageDetailMock.mockImplementation(serve());

    const wrapper = await mountPage();

    expect(fetchButton(wrapper)).toBeDefined();
    expect(wrapper.text()).not.toContain("Sign in to fetch this version");
  });

  /**
   * Signing in would not help on a registry kind that has no single artifact per
   * version, so the kind's own reason is what the row shows — the server keeps
   * sending it to a session-less reader for exactly this case.
   */
  it("shows the kind's reason rather than a sign-in prompt when signing in would not help", async () => {
    authState.isAuthenticated = false;
    explorePackageDetailMock.mockImplementation(
      serve({
        fetch: { offered: false, reason: "maven artifacts are a set of files" },
      }),
    );

    const wrapper = await mountPage();

    expect(wrapper.text()).toContain("maven artifacts are a set of files");
    expect(wrapper.text()).not.toContain("Sign in to fetch this version");
  });
});

/**
 * Pre-releases: not the default selection, and not on screen unless asked for.
 *
 * The endpoint sorts stable-before-pre-release, so `versions[0]` was already a
 * release whenever one existed — but the *held* preference cut across that: an
 * instance holding only `2.0.0-beta.1` opened on the beta while the releases sat
 * below it, and the README panel followed. Stable now wins the first pass and
 * "what we hold" decides within it.
 */
describe("PackageDetailPage pre-releases", () => {
  /** Sorted the way `explore/detail.rs` sorts: stable first, newest first. */
  const mixed = [
    version({ version: "3.0.0" }),
    version({ version: "2.0.0", source: "proxied" }),
    version({ version: "1.0.0", source: "proxied" }),
    version({ version: "4.0.0-rc.2", is_prerelease: true, source: "proxied" }),
    version({ version: "4.0.0-rc.1", is_prerelease: true }),
  ];

  const rows = (w: Awaited<ReturnType<typeof mountPage>>) =>
    w.findAll("tbody tr").map((r) => r.text());
  const selectedRow = (w: Awaited<ReturnType<typeof mountPage>>) =>
    w.findAll("tbody tr").find((r) => r.attributes("aria-current") === "true");
  const toggle = (w: Awaited<ReturnType<typeof mountPage>>) =>
    w.findAll("button").find((b) => /pre-release/i.test(b.text()));

  beforeEach(() => {
    routeState.query = {};
    replaceMock.mockReset();
    explorePackageDetailMock.mockReset().mockImplementation(serve({ versions: mixed }));
    listRegistriesMock.mockReset().mockResolvedValue({ data: [] });
    packageDetailMock.mockReset().mockResolvedValue({ data: undefined });
  });

  afterEach(() => {
    routeState.query = {};
  });

  it("opens on the newest held release, not on a newer held pre-release", async () => {
    const wrapper = await mountPage();

    expect(selectedRow(wrapper)!.text()).toContain("2.0.0");
    expect(wrapper.findComponent({ name: "ReadmePanel" }).props("version")).toBe("2.0.0");
  });

  it("prefers a release we do not hold over a pre-release we do", async () => {
    // Nothing stable is held: 3.0.0 is upstream-only and 4.0.0-rc.2 is proxied.
    explorePackageDetailMock.mockImplementation(
      serve({
        versions: [
          version({ version: "3.0.0" }),
          version({ version: "4.0.0-rc.2", is_prerelease: true, source: "proxied" }),
        ],
      }),
    );

    const wrapper = await mountPage();

    expect(selectedRow(wrapper)!.text()).toContain("3.0.0");
  });

  /** A package that has never cut a release still has to select something. */
  it("falls back to a pre-release when the package has no release at all", async () => {
    explorePackageDetailMock.mockImplementation(
      serve({
        versions: [
          version({ version: "1.0.0-beta.2", is_prerelease: true }),
          version({ version: "1.0.0-beta.1", is_prerelease: true, source: "proxied" }),
        ],
      }),
    );

    const wrapper = await mountPage();

    expect(selectedRow(wrapper)!.text()).toContain("1.0.0-beta.1");
  });

  it("hides pre-release rows until they are asked for", async () => {
    const wrapper = await mountPage();

    expect(rows(wrapper)).toHaveLength(3);
    expect(rows(wrapper).join(" ")).not.toContain("4.0.0-rc");
    expect(toggle(wrapper)!.text()).toContain("2");

    await toggle(wrapper)!.trigger("click");

    expect(rows(wrapper)).toHaveLength(5);
    expect(rows(wrapper).join(" ")).toContain("4.0.0-rc.2");
    expect(toggle(wrapper)!.text()).toMatch(/hide/i);
  });

  /**
   * A link naming a pre-release shows it, filter or no filter — the page must
   * never mark a row it does not draw.
   */
  it("shows the selected pre-release even while the rest stay hidden", async () => {
    routeState.query = { version: "4.0.0-rc.1" };

    const wrapper = await mountPage();

    expect(selectedRow(wrapper)!.text()).toContain("4.0.0-rc.1");
    expect(rows(wrapper).join(" ")).not.toContain("4.0.0-rc.2");
    // One of the two pre-releases is on screen, so the control offers the other.
    expect(toggle(wrapper)!.text()).toContain("1");
  });

  it("offers no control for a package that has no pre-releases", async () => {
    explorePackageDetailMock.mockImplementation(
      serve({ versions: [version({ version: "1.0.0", source: "proxied" })] }),
    );

    const wrapper = await mountPage();

    expect(toggle(wrapper)).toBeUndefined();
  });
});

/**
 * A long version list: filtered, paged, and still able to point at the row a
 * link named.
 *
 * `chalk` ships 44 versions and `@babel/plugin-transform-runtime` 169; the table
 * drew all of them. The two controls answer different questions — the filter
 * answers "is 4.0.2 here", the pager answers "how much of this is there" — so
 * they are asserted separately, and together where they interact.
 */
describe("PackageDetailPage version list", () => {
  /** 60 stable versions, newest first, the way the endpoint sorts them. */
  const many = Array.from({ length: 60 }, (_, i) =>
    version({ version: `1.${59 - i}.0`, source: i === 0 ? "proxied" : "upstream" }),
  );

  const rows = (w: Awaited<ReturnType<typeof mountPage>>) => w.findAll("tbody tr");
  const nextButton = (w: Awaited<ReturnType<typeof mountPage>>) =>
    w.findAll("button").find((b) => /next/i.test(b.text()));

  beforeEach(() => {
    routeState.query = {};
    replaceMock.mockReset();
    explorePackageDetailMock.mockReset().mockImplementation(serve({ versions: many }));
    listRegistriesMock.mockReset().mockResolvedValue({ data: [] });
    packageDetailMock.mockReset().mockResolvedValue({ data: undefined });
  });

  afterEach(() => {
    routeState.query = {};
  });

  it("draws one page rather than every version", async () => {
    const wrapper = await mountPage();

    expect(rows(wrapper)).toHaveLength(25);
    expect(wrapper.text()).toContain("1 of 3");
  });

  it("turns the page", async () => {
    const wrapper = await mountPage();
    expect(rows(wrapper)[0].text()).toContain("1.59.0");

    await nextButton(wrapper)!.trigger("click");

    expect(rows(wrapper)[0].text()).toContain("1.34.0");
    expect(wrapper.text()).toContain("2 of 3");
  });

  it("filters on the version string and says how much it kept", async () => {
    const wrapper = await mountPage();

    await typeFilter(wrapper, "1.5");

    // 1.5.0 and 1.50.0 … 1.59.0.
    expect(rows(wrapper)).toHaveLength(11);
    expect(wrapper.text()).toContain("11 of 60 shown");
  });

  /**
   * An empty result must not read as a package with no versions.
   *
   * It used to be an empty table, distinguishable only by the counter above it.
   * Once the rows are one page of a server-side filter, "no rows" is also the
   * shape of a package nothing has ever been pulled through — so the two
   * absences say which they are, in their own words.
   */
  it("says nothing matched rather than describing an empty package", async () => {
    const wrapper = await mountPage();

    await typeFilter(wrapper, "9.9.9");

    expect(wrapper.text()).toContain("0 of 60 shown");
    expect(wrapper.text()).toContain("No version matches");
    expect(wrapper.text()).not.toContain("No versions yet");
  });

  /** Filtering while on page 3 would otherwise land on an empty page. */
  it("returns to the first page when the filter changes", async () => {
    const wrapper = await mountPage();
    await nextButton(wrapper)!.trigger("click");
    expect(wrapper.text()).toContain("2 of 3");

    await typeFilter(wrapper, "1.1");

    // 11 matches fit on one page, so the pager goes away entirely — and the
    // rows shown are the first of the *filtered* list, not what page 2 held.
    expect(rows(wrapper)).toHaveLength(11);
    expect(rows(wrapper)[0].text()).toContain("1.19.0");
    expect(nextButton(wrapper)).toBeUndefined();
  });

  /**
   * A link to a version sixty rows down opens on the page that holds it —
   * otherwise the marked row is on a page the reader never sees, and the link
   * looks broken.
   */
  it("opens on the page holding the version the link names", async () => {
    routeState.query = { version: "1.4.0" };

    const wrapper = await mountPage();

    expect(wrapper.text()).toContain("3 of 3");
    expect(
      rows(wrapper)
        .find((r) => r.attributes("aria-current") === "true")!
        .text(),
    ).toContain("1.4.0");
  });
});

/**
 * What the page asks the endpoint for.
 *
 * The filter, the pager and the pre-release toggle stopped being this
 * component's the moment the answer became one page: a filter applied to the
 * rows in hand would answer "is 4.0.2 here" with *no* about a version this
 * server holds. These are the tests that the controls are wired to the request
 * rather than to a local array — assert the query, and assert that what comes
 * back is taken at its word rather than recomputed.
 */
describe("PackageDetailPage version request", () => {
  const many = Array.from({ length: 60 }, (_, i) =>
    version({ version: `1.${59 - i}.0`, source: i === 0 ? "proxied" : "upstream" }),
  );
  const lastQuery = () =>
    (explorePackageDetailMock.mock.calls.at(-1)?.[0]?.query ?? {}) as Record<string, unknown>;

  beforeEach(() => {
    routeState.query = {};
    replaceMock.mockReset();
    explorePackageDetailMock.mockReset().mockImplementation(serve({ versions: many }));
    listRegistriesMock.mockReset().mockResolvedValue({ data: [] });
    packageDetailMock.mockReset().mockResolvedValue({ data: undefined });
  });

  afterEach(() => {
    routeState.query = {};
  });

  it("asks for the rows it draws, releases only", async () => {
    await mountPage();

    expect(lastQuery().per_page).toBe(25);
    expect(lastQuery().prereleases).toBe("hide");
  });

  it("sends the filter rather than applying it to the rows in hand", async () => {
    const wrapper = await mountPage();

    await typeFilter(wrapper, "1.5");

    expect(lastQuery().q).toBe("1.5");
    expect(lastQuery().page).toBe(0);
  });

  /** One request for a word, not one per letter. */
  it("coalesces a burst of typing into one request", async () => {
    const wrapper = await mountPage();
    const before = explorePackageDetailMock.mock.calls.length;

    await wrapper.find("input").setValue("1");
    await wrapper.find("input").setValue("1.5");
    await wrapper.find("input").setValue("1.55");
    await new Promise((resolve) => setTimeout(resolve, 350));
    await flushPromises();

    expect(explorePackageDetailMock.mock.calls.length - before).toBe(1);
    expect(lastQuery().q).toBe("1.55");
  });

  /**
   * A slow answer for `1.` must not land on top of a fast one for `1.55`. The
   * request is the source of truth now, so the *last one sent* is the only one
   * whose answer is still true.
   */
  it("ignores an answer that a later request has superseded", async () => {
    const wrapper = await mountPage();
    const rules = serve({ versions: many });
    explorePackageDetailMock.mockImplementation((opts: { query?: Record<string, string> }) => {
      const slow = opts.query?.q === "1.";
      return rules(opts).then(
        (res) => new Promise((resolve) => setTimeout(() => resolve(res), slow ? 80 : 0)),
      );
    });

    await typeFilter(wrapper, "1.");
    await typeFilter(wrapper, "1.55");
    await new Promise((resolve) => setTimeout(resolve, 150));
    await flushPromises();

    // 1.55.0 alone, not the 60 the slow answer carried.
    expect(wrapper.text()).toContain("1 of 60 shown");
  });

  it("sends the page the reader turned to", async () => {
    const wrapper = await mountPage();

    await wrapper
      .findAll("button")
      .find((b) => /next/i.test(b.text()))!
      .trigger("click");
    await flushPromises();

    expect(lastQuery().page).toBe(1);
  });

  /**
   * A link naming a version asks for *no* page: only the server knows which one
   * holds it, and the answer says which it was.
   */
  it("lets the server choose the page for a link that names a version", async () => {
    routeState.query = { version: "1.4.0" };

    const wrapper = await mountPage();

    expect(lastQuery().page).toBeUndefined();
    expect(lastQuery().version).toBe("1.4.0");
    expect(wrapper.text()).toContain("3 of 3");
  });

  /** The clamp is the server's, and the URL is corrected to what it applied. */
  it("takes the page back from the server rather than keeping what it asked for", async () => {
    routeState.query = { page: "99" };
    replaceMock.mockImplementation((loc: { query?: Record<string, string> }) => {
      routeState.query = loc.query ?? {};
    });

    const wrapper = await mountPage();

    expect(wrapper.text()).toContain("3 of 3");
    expect(replaceMock).toHaveBeenLastCalledWith({
      path: "/packages/npm1/express",
      query: { page: "3" },
    });
  });

  /** The toggle is a request too, and an undebounced one — a click is one intent. */
  it("asks for the pre-releases when the control is pressed", async () => {
    const mixed = [
      version({ version: "4.0.0", source: "proxied" }),
      version({ version: "4.1.0-rc.1", is_prerelease: true }),
    ];
    explorePackageDetailMock.mockReset().mockImplementation(serve({ versions: mixed }));
    const wrapper = await mountPage();

    await wrapper
      .findAll("button")
      .find((b) => /show 1 pre-release/i.test(b.text()))!
      .trigger("click");
    await flushPromises();

    expect(lastQuery().prereleases).toBe("show");
    expect(wrapper.text()).toContain("4.1.0-rc.1");
  });
});

/**
 * The filter and the page are in the URL too.
 *
 * RFC 0013 §11 O1 said no to this — a version is a destination someone sends, a
 * page is a position in a session — and the position turned out to be worth
 * sending as well: "the four 4.0.x builds" and "the page the 2019 releases are
 * on" survived neither a reload, nor the page's own Refresh, nor being pasted to
 * a colleague. The keys are the catalog's, so the two pages read the same way:
 * `q` for the filter, `page` 1-based, both omitted at their default.
 */
describe("PackageDetailPage filter and page in the url", () => {
  const many = Array.from({ length: 60 }, (_, i) =>
    version({ version: `1.${59 - i}.0`, source: i === 0 ? "proxied" : "upstream" }),
  );

  const rows = (w: Awaited<ReturnType<typeof mountPage>>) => w.findAll("tbody tr");
  const filterInput = (w: Awaited<ReturnType<typeof mountPage>>) => w.find("input");
  const pagerButton = (w: Awaited<ReturnType<typeof mountPage>>, label: RegExp) =>
    w.findAll("button").find((b) => label.test(b.text()));

  beforeEach(() => {
    routeState.query = {};
    // Writes back, so a test can assert what the *next* load would see rather
    // than only what was asked for.
    replaceMock.mockReset().mockImplementation((loc: { query?: Record<string, string> }) => {
      routeState.query = loc.query ?? {};
    });
    explorePackageDetailMock.mockReset().mockImplementation(serve({ versions: many }));
    listRegistriesMock.mockReset().mockResolvedValue({ data: [] });
    packageDetailMock.mockReset().mockResolvedValue({ data: undefined });
  });

  afterEach(() => {
    routeState.query = {};
  });

  it("writes the filter into the url", async () => {
    const wrapper = await mountPage();

    await typeFilter(wrapper, "1.5");

    expect(replaceMock).toHaveBeenLastCalledWith({
      path: "/packages/npm1/express",
      query: { q: "1.5" },
    });
  });

  /** 1-based, because it is the number the pager shows a human. */
  it("writes the page into the url, and drops it again on page one", async () => {
    const wrapper = await mountPage();

    await pagerButton(wrapper, /next/i)!.trigger("click");
    expect(replaceMock).toHaveBeenLastCalledWith({
      path: "/packages/npm1/express",
      query: { page: "2" },
    });

    await pagerButton(wrapper, /previous|prev/i)!.trigger("click");
    expect(replaceMock).toHaveBeenLastCalledWith({
      path: "/packages/npm1/express",
      query: {},
    });
  });

  /**
   * Hydrating the filter must not trip the reset that a *typed* filter fires.
   * The two are the same state change and only the gesture tells them apart —
   * a watcher could not, and would take the page away on every load of a link
   * that named both.
   */
  it("opens on the filter and the page the link names", async () => {
    routeState.query = { q: "1", page: "2" };

    const wrapper = await mountPage();

    expect(filterInput(wrapper).element.value).toBe("1");
    expect(wrapper.text()).toContain("2 of 3");
    expect(rows(wrapper)[0].text()).toContain("1.34.0");
  });

  /**
   * An explicit page outranks the jump to the selected version, which would
   * otherwise pull the reader to page 1 on arrival — the page the default
   * selection sits on.
   */
  it("stays on the page the link names rather than the selection's", async () => {
    routeState.query = { page: "2" };

    const wrapper = await mountPage();

    expect(wrapper.text()).toContain("2 of 3");
  });

  it("does not follow a version to its own page when the url named another", async () => {
    routeState.query = { version: "1.4.0", page: "2" };

    const wrapper = await mountPage();

    expect(wrapper.text()).toContain("2 of 3");
  });

  /** Hand-edited, or a link sent before versions were yanked. */
  it("clamps a page past the end of the list", async () => {
    routeState.query = { page: "99" };

    const wrapper = await mountPage();

    expect(wrapper.text()).toContain("3 of 3");
    expect(rows(wrapper)).toHaveLength(10);
  });

  /** Refresh re-reads the list; it must not walk the reader back to page one. */
  it("keeps the reader's page across a refresh", async () => {
    routeState.query = { page: "2" };
    const wrapper = await mountPage();

    await wrapper
      .findAll("button")
      .find((b) => b.text().includes("Refresh"))!
      .trigger("click");
    await flushPromises();

    expect(wrapper.text()).toContain("2 of 3");
  });

  /** Typing is still a gesture, and a gesture still starts the list over. */
  it("drops the page from the url when the filter is typed", async () => {
    const wrapper = await mountPage();
    await pagerButton(wrapper, /next/i)!.trigger("click");

    await typeFilter(wrapper, "1.1");

    expect(replaceMock).toHaveBeenLastCalledWith({
      path: "/packages/npm1/express",
      query: { q: "1.1" },
    });
  });
});
