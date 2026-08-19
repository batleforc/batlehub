<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { ref, computed, onMounted, onBeforeUnmount, watch } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";
import {
  ArrowLeft,
  ShieldCheck,
  ShieldAlert,
  Lock,
  Unlock,
  FileJson,
  FileCode,
  Download,
} from "@lucide/vue";
import { explorePackageDetail, exploreFetchVersion, listRegistries } from "@/client/sdk.gen";
import type { ExplorePackageDetailResponse, FirewallDto, RegistryInfo } from "@/client/types.gen";
import { useAuth } from "@/composables/useAuth";
import { packageDetail } from "@/client/sdk.gen";
import type { PackageDetailResponse } from "@/client/types.gen";
import PackageVersionsTable from "@/components/admin/PackageVersionsTable.vue";
import PackageBetaChannel from "@/components/admin/PackageBetaChannel.vue";
import PackageVisibility from "@/components/admin/PackageVisibility.vue";
import PackageEventsTable from "@/components/admin/PackageEventsTable.vue";
import ReadmePanel from "@/components/package/ReadmePanel.vue";
import UpstreamNotice from "@/components/package/UpstreamNotice.vue";
import { Separator } from "@/components/ui/separator";
import { useAuthFetch } from "@/composables/useAuthFetch";
import { useApi, extractMessage } from "@/composables/useApi";
import { API_BASE_URL } from "@/config";
import { formatBytes, formatCount } from "@/lib/format";
import { severityVariant } from "@/lib/badge-variants";
import { Badge } from "@/components/ui/badge";
import { Resolution, type ResolutionState } from "@/components/ui/resolution";
import { EmptyState } from "@/components/ui/empty-state";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Pagination } from "@/components/ui/pagination";
import { Search } from "@lucide/vue";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableHeader,
  TableHead,
  TableBody,
  TableRow,
  TableCell,
} from "@/components/ui/table";

const { t } = useI18n();

const { token, isAdmin, isAuthenticated } = useAuth();
const { authFetch } = useAuthFetch();
const route = useRoute();
const router = useRouter();

const registry = computed(() => String(route.params.registry ?? ""));
const name = computed(() => String(route.params.name ?? ""));

const { data: registriesList } = useApi<RegistryInfo[]>(
  () => listRegistries() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);
const registryType = computed(
  () => registriesList.value?.find((r) => r.name === registry.value)?.type ?? null,
);

const data = ref<ExplorePackageDetailResponse | null>(null);
const loading = ref(false);
const error = ref<string | null>(null);

/**
 * The version the README panel follows and the versions table marks.
 *
 * `null` only while the detail request is in flight, and for a package with no
 * versions at all; the endpoint's own `selected_version`/`default_version` set
 * it, the latter preferring the newest version this instance actually holds. A
 * `null` still means "newest that has a README" to the endpoint, which is the
 * right answer for the one case that reaches it.
 */
const selectedVersion = ref<string | null>(null);

/**
 * Whether a row describes a version this instance holds no bytes for.
 *
 * Every cell that would be a fact about what we hold reads *unknown* on such a
 * row rather than `0` or `—`: nobody has downloaded it *through here*, which is
 * not the same as nobody having downloaded it (RFC 0007 §4.2).
 */
function isUpstreamOnly(source: string) {
  return source === "upstream";
}

// What the page selects when the reader has selected nothing — **the newest
// stable version this instance holds** — is `default_version` in the response
// now, and the rule lives in `explore/detail.rs`.
//
// It was this component's, over the whole list, and the rule does not survive
// being handed one page of that list: "the first held stable row" read off page
// one of a package held only at 2.1.0 picks an upstream row, which is the exact
// defect RFC 0007 §4.2 wrote the rule to fix. One home for it, and the home is
// the side that can see every version.

// ── Pre-releases are opt-in ───────────────────────────────────────────────────
//
// A release candidate is not what a reader is looking for by default, and on a
// package like `chalk` the pre-releases outnumber the releases at the top of the
// list — so the first screenful of the table was versions nobody asked about.
// Hidden by default, one control to show them, and the count says how many are
// behind it: a filter that does not say what it removed reads as a short list
// rather than as a filtered one.
const showPrereleases = ref(false);

// ── Filtering and paging a long version list ─────────────────────────────────
//
// `chalk` ships 44 versions, `@babel/plugin-transform-runtime` 169, and the
// table drew every one of them: a page whose useful content — the gate, the
// README — sat above a list nobody scrolls to the end of, and a browser laying
// out a hundred-odd rows of buttons and badges on every render.
//
// A filter and a pager rather than a "show more": the question a reader has in
// front of a long version list is "is 4.0.2 here", and scrolling is a poor
// answer to it.
//
// **All three questions are the server's now** — the filter, the page, and which
// pre-releases to include. They were this component's, over a list the endpoint
// sent whole, and that arrangement had the endpoint building 169 rows (a
// vulnerability read and an SBOM read each) so the browser could draw 25 of
// them. Worse, once the answer is a *page*, a filter applied to it would search
// only what happened to arrive: "is 4.0.2 here" would be answered *no* about a
// version this server knows perfectly well it has. So the controls send, and
// what comes back is already the answer (RFC 0013 §4.3).
const versionFilter = ref("");
const versionPage = ref(0);
/** Set by `applyQuery`, consumed by the first fetch: see `versionQuery`. */
let letServerChoosePage = false;
/** What this page asks for. The operator's `[limits].versions_per_page` is the
    ceiling, not this: the console asks for the number of rows it draws, and
    reads back what the server actually applied. */
const VERSIONS_PER_PAGE = 25;

/** The rows the table draws — one page of them, exactly as sent. */
const pagedVersions = computed(() => data.value?.versions ?? []);

/** The page arithmetic, from the server's totals rather than from the rows in
    hand, which are one page and cannot say how many there are. */
const versionsPerPage = computed(() => data.value?.versions_page?.per_page ?? VERSIONS_PER_PAGE);
const filteredTotal = computed(() => data.value?.versions_page?.total ?? 0);
const unfilteredTotal = computed(() => data.value?.versions_page?.unfiltered_total ?? 0);
const prereleaseCount = computed(() => data.value?.versions_page?.prerelease_total ?? 0);
const hiddenPrereleaseCount = computed(() => data.value?.versions_page?.hidden_prereleases ?? 0);

const versionTotalPages = computed(() =>
  Math.max(1, Math.ceil(filteredTotal.value / versionsPerPage.value)),
);

/**
 * Upstream-only rows, for the notice's count.
 *
 * Counted over the rows on screen: the notice says "N version(s) below exist
 * upstream and are not held here", and `below` is the table as drawn. Promising
 * rows a reader would have to turn a page to find would be the same defect in a
 * new place.
 */
const upstreamVersionCount = computed(
  () => pagedVersions.value.filter((v) => v.source === "upstream").length,
);

/*
 * Both controls change *what the pages contain*, so both start over at the
 * first one: a filter that left you on page 4 of a two-page result would look
 * like an empty list.
 *
 * The reset hangs off the gesture rather than off a watcher on the state,
 * because the state has a second author now — the URL. Hydrating `?q=1.5&page=2`
 * sets the filter, and a watcher could not tell that apart from a keystroke, so
 * it would take the page away again on every load of a link that named both.
 */
function filterVersions(value: string): void {
  versionFilter.value = value;
  versionPage.value = 0;
  // Debounced, because this now costs a request. The pager and the pre-release
  // toggle are not: a click is one intent, and a delay after one reads as the
  // control not working.
  scheduleVersionFetch();
}

function togglePrereleases(): void {
  showPrereleases.value = !showPrereleases.value;
  versionPage.value = 0;
  void fetchVersions();
}

function turnToPage(page: number): void {
  versionPage.value = page;
  void fetchVersions();
}

// ── The version, the filter and the page live in the URL ──────────────────────
//
// A version is a destination: "look at 4.0.2 of this package" is a thing one
// person sends another, and it was unsendable — the selection was component
// state, so every link to this page landed on whatever the page chose for
// itself. It also did not survive the page's own Refresh, which re-derived the
// default and discarded whatever the reader had opened.
//
// The filter and the page followed, for the same reason one step further out. A
// long version list is read by narrowing it, and "the four 4.0.x builds of this
// package" or "the page where the 2019 releases are" was a position that existed
// only inside the tab that produced it: reload, Refresh, or paste the address to
// a colleague, and they got the whole list from the top. RFC 0013 §11 O1 argued
// the other way — a version is a destination, a page is a position in a session
// — and the position turned out to be worth sending too.
//
// The rule is the catalog's, so the two pages read the same way and use the same
// key names: the query carries a value **only when it is not the default one**,
// defaults are omitted, the page is 1-based because a human reads it, and an
// unrecognised value falls back to the default rather than to nothing. A version
// this package does not have — a typo, or one yanked since the link was sent —
// is exactly that case, so the page opens on its default instead of marking no
// row at all.
//
// `replace`, never `push`: reading down a version list is not ten destinations,
// and a keystroke in the filter is not one either. Pushing would make Back walk
// through every row the reader clicked and every letter they typed before it
// returned them to the catalog — the journey RFC 0003 §9's back button exists to
// make short.

/** The version the URL asks for, as asked — whether this package has it is the
    server's answer, not one this page can give from a single page of rows. */
function versionFromQuery(): string | null {
  const asked = Array.isArray(route.query.version) ? route.query.version[0] : route.query.version;
  return typeof asked === "string" && asked ? asked : null;
}

/**
 * Read the filter and the page back off the URL.
 *
 * The version is not read here: it is sent to the endpoint, which answers with
 * `selected_version` — the ask, if this package has it — because a typo or a
 * version yanked since the link was sent cannot be told apart from a version on
 * another page by anything holding one page.
 *
 * Anything absent or unrecognised falls to the default the ref is declared with,
 * so a hand-edited URL cannot put the page in a state its own controls could not
 * produce.
 */
function applyQuery(): void {
  const one = (v: unknown) => (Array.isArray(v) ? v[0] : v);

  const term = one(route.query.q);
  versionFilter.value = typeof term === "string" ? term : "";

  const p = Number(one(route.query.page));
  versionPage.value = Number.isFinite(p) && p > 1 ? Math.floor(p) - 1 : 0;
  // The one request where the server picks the page: a link that named a version
  // and no page opens on the page holding it, and only the server can say which
  // that is. Every later request sends the page the reader is on — including
  // page 1, because a page turned back to is a choice.
  letServerChoosePage = one(route.query.page) === undefined;
}

/** What this page's state serialises to. Keys it does not own are carried
    through untouched — it writes its three and reads back everything. */
function stateAsQuery(): Record<string, unknown> {
  const { version: _v, q: _q, page: _p, ...rest } = route.query;
  const next: Record<string, unknown> = { ...rest };

  const isDefault = selectedVersion.value === (data.value?.default_version ?? null);
  if (selectedVersion.value && !isDefault) next.version = selectedVersion.value;
  if (versionFilter.value.trim()) next.q = versionFilter.value;
  if (versionPage.value > 0) next.page = String(versionPage.value + 1); // 1-based for a human

  return next;
}

function syncQuery(): void {
  // Only ever writes to a package's own route: a detail request settling after
  // the reader has moved on must not rewrite the address they moved to.
  if (!route.path.startsWith("/packages/")) return;

  const next = stateAsQuery();
  const current = route.query;
  const same =
    Object.keys(next).length === Object.keys(current).length &&
    Object.entries(next).every(([k, v]) => current[k] === v);
  // Vue Router rejects a redundant navigation, and writing one per keystroke
  // would churn the address bar for no change.
  if (!same) void router.replace({ path: route.path, query: next as Record<string, string> });
}

watch([selectedVersion, versionFilter, versionPage], syncQuery);

// ── Per-artifact SBOM download ─────────────────────────────────────────────

const sbomLoading = ref<string | null>(null); // "registry/name/version:format"
const sbomMissing = ref<Set<string>>(new Set());

async function downloadSbom(version: string, fmt: "spdx" | "cyclonedx") {
  const key = `${registry.value}/${name.value}/${version}:${fmt}`;
  sbomLoading.value = key;
  try {
    const ext = fmt === "cyclonedx" ? "cyclonedx.json" : "spdx.json";
    const url = `/api/v1/sbom/${encodeURIComponent(registry.value)}/${encodeURIComponent(name.value)}/${encodeURIComponent(version)}?format=${fmt}`;
    const resp = await authFetch(`${API_BASE_URL}${url}`);
    if (resp.status === 404) {
      sbomMissing.value = new Set([
        ...sbomMissing.value,
        `${registry.value}/${name.value}/${version}`,
      ]);
      return;
    }
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    const disposition = resp.headers.get("Content-Disposition") ?? "";
    const match = disposition.match(/filename="([^"]+)"/);
    const filename = match?.[1] ?? `${name.value}-${version}.${ext}`;
    const blob = await resp.blob();
    const a = Object.assign(document.createElement("a"), {
      href: URL.createObjectURL(blob),
      download: filename,
    });
    a.click();
    URL.revokeObjectURL(a.href);
  } catch {
    // silently ignore download errors
  } finally {
    sbomLoading.value = null;
  }
}

/**
 * Which version is being fetched, and what came of the last attempt
 * (RFC 0007-bis §4.4).
 *
 * Keyed by version rather than a single flag: a reader may press one row while
 * reading another, and a shared spinner would put the wrong row in motion.
 */
const fetching = ref<string | null>(null);
const fetchResult = ref<Record<string, string>>({});

/**
 * Ask this instance to fetch one version from upstream.
 *
 * Synchronous, with a spinner. Measured against real upstreams the median
 * version is 0.57 MB in 66 ms and the largest sampled was 41.7 MB in 417 ms
 * (RFC 0007-bis §13.4), which a spinner holds comfortably — and the response
 * reports the size, so the row can say what the wait bought rather than just
 * "done".
 */
async function onFetchVersion(version: string) {
  if (fetching.value) return;
  fetching.value = version;
  delete fetchResult.value[version];
  try {
    const { data: res, error: apiErr } = await exploreFetchVersion({
      path: { registry: registry.value, name: name.value, version },
    });
    if (apiErr) {
      // The rule's own reason, shown verbatim. It is the same string the
      // download would have given, so the operator can take it to the RBAC
      // simulator and get the same verdict explained.
      const body = apiErr as { code?: string; message?: string };
      fetchResult.value[version] =
        body.code === "fetch.already-held"
          ? t("packageDetailPage.fetchAlreadyHeld")
          : body.message
            ? t("packageDetailPage.fetchDenied", { reason: body.message })
            : t("packageDetailPage.fetchFailed");
      return;
    }
    const size = (res as { size_bytes?: number }).size_bytes ?? 0;
    fetchResult.value[version] = t("packageDetailPage.fetched", {
      size: formatBytes(size),
    });
    // Refresh so the row says `proxied` rather than leaving the reader to
    // reload and wonder whether it worked.
    await fetchDetail();
  } catch (e) {
    fetchResult.value[version] = extractMessage(e);
  } finally {
    fetching.value = null;
  }
}

/**
 * What the version controls send.
 *
 * `version` goes on every request, not only when a link named one: it is what
 * keeps a selected pre-release in a list that hides them, and what lets the
 * endpoint answer with the page holding it.
 */
function versionQuery(): Record<string, string | number> {
  const q: Record<string, string | number> = {
    per_page: VERSIONS_PER_PAGE,
    prereleases: showPrereleases.value ? "show" : "hide",
  };
  if (!letServerChoosePage) q.page = versionPage.value;
  if (versionFilter.value.trim()) q.q = versionFilter.value.trim();
  const pinned = selectedVersion.value ?? versionFromQuery();
  if (pinned) q.version = pinned;
  return q;
}

/**
 * The one fetch, in two moods.
 *
 * `silent` is what the version controls use: they change one card on a page
 * whose other half is a README, and flipping the whole page into its loading
 * state for a keystroke would blank that README, drop the pager, and take the
 * focus out of the filter box the reader is still typing in. The rows are
 * simply replaced when the answer lands.
 *
 * `seq` because a filter is typed: a slow answer for `1.` must never overwrite
 * a fast one for `1.55`, and the last request sent is the only one whose answer
 * is still the truth.
 */
let versionSeq = 0;

async function fetchDetail(opts: { silent?: boolean } = {}) {
  const seq = ++versionSeq;
  if (!opts.silent) {
    loading.value = true;
    error.value = null;
  }
  try {
    const { data: res, error: apiErr } = await explorePackageDetail({
      path: { registry: registry.value, name: name.value },
      query: versionQuery(),
    });
    if (seq !== versionSeq) return; // superseded by a later keystroke or click
    if (apiErr) throw new Error(`HTTP error`);
    data.value = res as ExplorePackageDetailResponse;
    letServerChoosePage = false;
    // Read back rather than assume: the server clamps a page past the end, caps
    // `per_page` at the operator's ceiling, and picks the page itself when a
    // link named a version and no page.
    versionPage.value = data.value.versions_page?.page ?? 0;
    // An unknown version — a typo, or one yanked since the link was sent —
    // comes back as `selected_version: null`, and falls back rather than
    // marking no row at all.
    selectedVersion.value = data.value.selected_version ?? data.value.default_version ?? null;
  } catch (e) {
    if (seq === versionSeq) error.value = extractMessage(e);
  } finally {
    if (seq === versionSeq && !opts.silent) loading.value = false;
  }
}

/** A version-control change: same request, no page-wide loading state. */
function fetchVersions() {
  return fetchDetail({ silent: true });
}

let filterTimer: ReturnType<typeof setTimeout> | null = null;

/** Typing is debounced because it now costs a request; 300 ms, the catalog's
    number, so the two search boxes feel the same. */
function scheduleVersionFetch() {
  if (filterTimer) clearTimeout(filterTimer);
  filterTimer = setTimeout(() => {
    filterTimer = null;
    void fetchVersions();
  }, 300);
}

// Routing away within 300 ms of a keystroke would otherwise run the fetch
// against a destroyed component — the same footgun the catalog fixed.
onBeforeUnmount(() => {
  if (filterTimer) clearTimeout(filterTimer);
});

/**
 * Back to the catalog **as the reader left it**, not to a fresh one.
 *
 * This pushed `/packages?registry=…`, which looked like it preserved something
 * and did not: the catalog never read `route.query`, so the registry was dropped
 * on arrival along with the search, the scope, the sort and the page. Opening a
 * package from the fifth page of a search and pressing Back gave an empty search
 * box and page one.
 *
 * `router.back()` when the previous entry is the catalog, because that restores
 * the URL *and* the scroll offset (`scrollBehavior` in `router/index.ts` returns
 * the saved position on a pop) — and the catalog now carries its whole state in
 * the query, so the URL is the search. Rebuilding a location by hand would
 * reconstruct the query and still lose the offset.
 *
 * The fallback is for arriving without that history: a pasted link, a refresh, a
 * new tab. `history.state.back` is Vue Router's own record of the previous
 * entry, so this asks the question directly rather than guessing from
 * `document.referrer`. A detail page also starts with `/packages`, hence the
 * exact-path test — going "back" from one package to another would be a
 * surprise, not a return.
 */
function isCatalogEntry(entry: unknown): boolean {
  if (typeof entry !== "string") return false;
  return entry.split("?")[0].split("#")[0] === "/packages";
}

function goBack() {
  if (isCatalogEntry(window.history.state?.back)) {
    router.back();
    return;
  }
  router.push({ path: "/packages", query: { registry: registry.value } });
}

/**
 * These three returned English literals shipped past a green i18n audit: §4.1
 * taught the scanner about component props and `ref` assignments, and a string
 * literal returned from a function is neither. The rule it was supposed to learn
 * was "human-readable text is text that reaches a human, wherever it is
 * written", so this is the same class again, one position over.
 */
function firewallLabel(fw: FirewallDto) {
  if (fw.status === "blocked") return t("common.blocked");
  if (fw.status === "yanked") return t("packageDetailPage.firewallYanked");
  return t("packageDetailPage.firewallClear");
}

/**
 * Firewall status in DESIGN.md's resolution vocabulary. Three of the six states
 * map one-to-one: a clear version is held and verified, and blocked/yanked are
 * named identically in both.
 */
function firewallResolution(fw: FirewallDto): ResolutionState {
  if (fw.status === "blocked") return "blocked";
  if (fw.status === "yanked") return "yanked";
  return "cached";
}

function formatDate(iso: string | null) {
  if (!iso) return "—";
  return new Date(iso).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

// ── Download URL construction ──────────────────────────────────────────────────

/**
 * Build the proxy download URL for a given version based on the registry type.
 * Returns null for registries whose URL can't be derived purely from name/version
 * (Maven, PyPI simple page, etc.).
 */
function downloadUrl(version: string): string | null {
  const n = name.value;
  const r = registry.value;
  const base = `${API_BASE_URL}/proxy/${encodeURIComponent(r)}`;
  switch (registryType.value) {
    case "cargo":
      return `${base}/${encodeURIComponent(n)}/${encodeURIComponent(version)}/download`;
    case "npm":
      // encodeURIComponent handles scoped packages: @scope/pkg → %40scope%2Fpkg (one path segment)
      return `${base}/${encodeURIComponent(n)}/${encodeURIComponent(version)}/tarball`;
    case "nuget":
      return `${base}/nuget/v3/flat/${encodeURIComponent(n.toLowerCase())}/${encodeURIComponent(version.toLowerCase())}/${encodeURIComponent(n.toLowerCase())}.${encodeURIComponent(version.toLowerCase())}.nupkg`;
    case "rubygems":
      return `${base}/gems/${encodeURIComponent(n)}-${encodeURIComponent(version)}.gem`;
    case "pypi":
      // PyPI has hashed filenames — link to the simple page instead
      return `${base}/simple/${encodeURIComponent(n)}/`;
    case "conda":
      return `${base}/noarch/${encodeURIComponent(n)}-${encodeURIComponent(version)}-py_0.conda`;
    case "vsix":
    case "openvsx": {
      const parts = n.split(".");
      if (parts.length === 2) {
        return `${base}/${encodeURIComponent(parts[0])}.${encodeURIComponent(parts[1])}/${encodeURIComponent(version)}/vsix`;
      }
      return null;
    }
    default:
      return null;
  }
}

onMounted(() => {
  // Before the request, not after: the query *is* the initial state, so the list
  // must arrive into the filter and the page the URL asked for. Hydrating
  // afterwards would draw the whole list from the top and then move it.
  applyQuery();
  void fetchDetail();
});

/**
 * Administration is a *section of this page* now, not a parallel page at another
 * URL with its own layout and its own back button. One package, one address
 * (RFC 0003 §4.2).
 *
 * Fetched only when the viewer is an admin: the endpoint is admin-only
 * server-side, so firing it for everyone would mean a 403 on every package view.
 * Hiding the section is a rendering decision — the server still refuses.
 */
const {
  data: adminData,
  error: adminError,
  reload: reloadAdmin,
} = useApi<PackageDetailResponse>(
  () =>
    isAdmin.value
      ? (packageDetail({
          query: { registry: registry.value, name: name.value },
        }) as Promise<{
          data?: unknown;
          error?: unknown;
        }>)
      : Promise.resolve({ data: undefined }),
  [token, registry, name],
);
</script>

<template>
  <!-- Full-bleed, like the masthead, the footer and the catalog.

       This was `max-w-4xl`: 896px of page no matter how wide the window, so at
       1920 the content stopped 1009px short of the rails the header and footer
       draw, and the versions table — nine columns, of which SECURITY, SBOM and
       DOWNLOAD are the ones an operator is here for — sat inside 846px and
       scrolled them out of sight. The measure was also only half applied: the
       whole Administration section below already renders *outside* this div and
       ran edge to edge, so an admin was reading one page drawn to two different
       widths.

       No page-level measure replaces it. That is the rule App.vue states when
       it drops the global container: a page sets no width, and the surfaces
       that need one set their own — the specimen caption at `72ch`,
       `PageHeader` and `EmptyState` at `64ch`, and now the README body, which
       is the only long-form prose here (see `ReadmePanel.vue`). A table and a
       paragraph do not want the same width, which is exactly what one cap on
       the page was giving them. -->
  <div class="space-y-6">
    <!-- Back link -->
    <button
      class="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors"
      @click="goBack"
    >
      <ArrowLeft class="h-4 w-4" />
      {{ t("packageDetailPage.backToCatalog") }}
    </button>

    <template v-if="loading">
      <p class="text-muted-foreground text-sm">
        {{ t("packageDetailPage.loading") }}
      </p>
    </template>

    <template v-else-if="error">
      <p class="text-destructive text-sm">{{ error }}</p>
    </template>

    <template v-else-if="data">
      <!-- The specimen, as the catalog builds it: the page announces its
           subject at the Display step, and the facts sit under it as a caption.

           DESIGN.md gives Display ("Silkscreen 700, 56 → 72 → 88 → 104 per
           breakpoint, line-height 0.92, 0.02em") to *the subject at the top of
           the sheet, one per view*. This h1 was `text-2xl font-mono` — 24px of
           JetBrains Mono — so the one page dedicated to a single package set its
           name in the reading face at a size the doc reserves for the wordmark,
           and the rendered gate reports `h1 not in the display face` at every
           width and role. The catalog spends this step on the registry; here the
           subject is the package, so it spends it on the package.

           **No `uppercase`, and the caption carries the coordinate.** Silkscreen
           is caps-only — `abg` and `ABG` rasterise to identical pixels, checked
           on a canvas rather than assumed from a screenshot — so the transform
           the catalog's specimen applies would be a decision that changes
           nothing here, and, more to the point, *the headline cannot show a
           package's case at all*. That is fine for a registry label (`NPM`) and
           not fine for a coordinate the reader copies into a manifest, where
           `React-Smooth` and `react-smooth` are two different packages on
           Maven, NuGet and Go. So the exact name is repeated in the caption in
           the text face, which is the one place on this page it was otherwise
           unavailable in its true form.

           `break-words`, **not** `[overflow-wrap:anywhere]` — the opposite of
           the choice the catalog's name *cell* documents, and the difference is
           the size. `anywhere` opens a break opportunity at every character, so
           at 390px `react-smooth` came out as `REACT` / `-` / `SMOOT` / `H`: the
           hyphen alone on one line and an orphan glyph on the next.
           `break-words` breaks as a last resort only, so the browser takes the
           hyphen first (`REACT-` / `SMOOTH`) and splits mid-glyph just when a
           segment cannot fit at all — which at 104px a name like
           `@babel/plugin-transform-runtime` does need. In a 15px table cell the
           min-content sizing is what matters and `anywhere` wins; in a 104px
           headline the break *quality* is what matters. `min-w-0` on the flex
           child is what lets either act.

           The icon and the registry `Badge` came out of the heading line. A
           24px lucide glyph and a bordered chip beside a 104px headline read as
           three competing objects, and the registry is a *fact about* the
           subject, not part of its name — so it joins the caption, in the
           order the catalog's own caption uses. -->
      <!-- A column until `sm`, a row above it. The button was a sibling of the
           specimen in one wrapping flex row, and `flex-1` + `min-w-0` means the
           *text* yields, not the button: at 390 the h1 got 255px of a 358px
           line, so a 56px headline broke as `REACT` / `-` / `SMOOT` / `H`
           inside a column two glyphs narrower than the page. Stacking gives the
           subject the whole line — `REACT-` / `SMOOTH` — and costs a control
           nothing, since Refresh is the same size wherever it sits. -->
      <div class="flex flex-col gap-3 sm:flex-row sm:items-start">
        <div class="min-w-0 flex-1">
          <h1
            class="font-display font-bold tracking-[0.02em] leading-[0.92] text-display break-words"
          >
            {{ data.name }}
          </h1>
          <p class="mt-3 text-sm text-muted-foreground [overflow-wrap:anywhere]">
            <span class="font-mono text-foreground">{{ data.name }}</span>
            <span class="px-2 text-border" aria-hidden="true">·</span>
            <span class="text-foreground">{{ data.registry }}</span>
            <span class="px-2 text-border" aria-hidden="true">·</span>
            {{ t("packageDetailPage.knownVersions", unfilteredTotal) }}
          </p>
        </div>
        <Button variant="outline" size="sm" @click="fetchDetail">
          {{ t("common.refresh") }}
        </Button>
      </div>

      <!-- Gate summary card -->
      <Card>
        <CardHeader class="pb-2">
          <CardTitle class="text-base">{{ t("packageDetailPage.accessGate") }}</CardTitle>
        </CardHeader>
        <CardContent>
          <div class="space-y-2">
            <!-- Registry access -->
            <div class="flex items-center gap-2 text-sm">
              <component
                :is="data.gate.registry_accessible ? ShieldCheck : ShieldAlert"
                :class="data.gate.registry_accessible ? 'text-primary' : 'text-destructive'"
                class="h-4 w-4 shrink-0"
              />
              <span class="text-muted-foreground">{{ t("packageDetailPage.registryAccess") }}</span>
              <span
                :class="
                  data.gate.registry_accessible
                    ? 'text-primary font-medium'
                    : 'text-destructive font-medium'
                "
              >
                {{
                  data.gate.registry_accessible ? t("accessCheck.allowed") : t("accessCheck.denied")
                }}
              </span>
            </div>

            <!-- Beta channel -->
            <div class="flex items-center gap-2 text-sm">
              <component
                :is="data.gate.beta_member ? Unlock : Lock"
                :class="data.gate.beta_member ? 'text-primary' : 'text-muted-foreground'"
                class="h-4 w-4 shrink-0"
              />
              <span class="text-muted-foreground">{{ t("packageDetailPage.betaChannel") }}</span>
              <span
                :class="
                  data.gate.beta_member ? 'text-primary font-medium' : 'text-muted-foreground'
                "
              >
                {{
                  data.gate.beta_member
                    ? t("packageDetailPage.memberPreReleaseVersionsVisible")
                    : t("packageDetailPage.nonMember")
                }}
              </span>
            </div>
          </div>
        </CardContent>
      </Card>

      <!-- README, below the header and above the versions table, bound to the
           selected version. Fetched separately from the detail response so the
           catalogue cache's TTL never holds a stale document and the detail
           payload does not grow by a megabyte per package (RFC 0007 §5.4). -->
      <ReadmePanel :registry="registry" :name="name" :version="selectedVersion" />

      <!-- Versions table -->
      <Card>
        <CardHeader class="pb-2">
          <div class="flex flex-wrap items-center justify-between gap-3">
            <CardTitle class="text-base">{{ t("common.versions") }}</CardTitle>
            <!-- The filter says what it is hiding. A control reading "Show
                 pre-releases" over a list that has none is a promise of
                 something to reveal, so it renders only when there is; and the
                 count is in the label rather than in a tooltip, because "8
                 hidden" is the fact that decides whether a reader clicks. -->
            <Button
              v-if="prereleaseCount > 0"
              variant="outline"
              size="sm"
              class="h-7 px-2 text-xs"
              :aria-pressed="showPrereleases"
              @click="togglePrereleases()"
            >
              {{
                showPrereleases
                  ? t("packageDetailPage.hidePrereleases", prereleaseCount)
                  : t("packageDetailPage.showPrereleases", hiddenPrereleaseCount)
              }}
            </Button>
          </div>
        </CardHeader>
        <CardContent class="p-0">
          <div v-if="data.upstream.attempted" class="px-4 pb-2">
            <UpstreamNotice
              :upstream="data.upstream"
              :upstream-version-count="upstreamVersionCount"
            />
          </div>
          <!-- The filter sits with the list it filters rather than in the card
               header with the pre-release control: that one changes *what the
               list is*, this one searches inside it, and putting them together
               would read as two halves of one filter. -->
          <div v-if="unfilteredTotal > 1" class="flex flex-wrap items-center gap-3 px-4 pb-3">
            <div class="relative min-w-48 flex-1">
              <Search class="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
              <Input
                class="pl-8"
                :placeholder="t('packageDetailPage.filterVersions')"
                :aria-label="t('packageDetailPage.filterVersions')"
                :value="versionFilter"
                @input="filterVersions(($event.target as HTMLInputElement).value)"
              />
            </div>
            <!-- Says what the list is showing out of what there is. A filter
                 whose result is empty must not be indistinguishable from a
                 package with no versions. -->
            <span class="text-xs text-muted-foreground" role="status" aria-live="polite">
              {{
                t("packageDetailPage.versionsShown", {
                  shown: filteredTotal,
                  total: unfilteredTotal,
                })
              }}
            </span>
          </div>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{{ t("common.version") }}</TableHead>
                <TableHead>{{ t("common.source") }}</TableHead>
                <TableHead>{{ t("common.firewall") }}</TableHead>
                <TableHead class="text-right">{{ t("common.downloads") }}</TableHead>
                <TableHead>{{ t("packageDetailPage.lastAccessed") }}</TableHead>
                <TableHead>{{ t("common.published") }}</TableHead>
                <TableHead>{{ t("common.security") }}</TableHead>
                <TableHead v-if="token">SBOM</TableHead>
                <TableHead>{{ t("common.download") }}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <!-- Selection rides on ink and a lit edge, never on fill.

                   It was `bg-muted/40`, and DESIGN.md's Undependable Fill Rule
                   is about exactly this: on this ground the raised fill computes
                   to about 1.06:1 against the sheet, so the marked row was
                   indistinguishable from every other one — the page chose a
                   default version and then had no way to say which. `Facet`
                   already solved this for the registry bay ("selection rides on
                   ink and a lit edge, not a fill"), and this is the same
                   statement in a table: a crimson left edge, the ink lifted to
                   `--ink` and the weight raised.

                   The transparent edge on unselected rows reserves the same 2px,
                   so marking a row moves nothing sideways.

                   `aria-current="true"` carries it to a screen reader, which
                   cannot see either the rule or the weight. -->
              <TableRow
                v-for="ver in pagedVersions"
                :key="`${ver.version}-${ver.source}`"
                :class="[
                  'cursor-pointer border-l-2',
                  selectedVersion === ver.version
                    ? 'border-l-primary font-semibold text-foreground'
                    : 'border-l-transparent',
                  ver.is_prerelease && selectedVersion !== ver.version
                    ? 'text-muted-foreground italic'
                    : '',
                  ver.is_prerelease && selectedVersion === ver.version ? 'italic' : '',
                ]"
                :aria-current="selectedVersion === ver.version ? 'true' : undefined"
                @click="selectedVersion = ver.version"
              >
                <TableCell class="font-mono text-sm">
                  {{ ver.version }}
                  <Badge v-if="ver.is_prerelease" variant="outline" class="ml-1 text-xs">
                    pre-release
                  </Badge>
                  <Badge
                    v-if="ver.deprecated"
                    variant="destructive"
                    class="ml-1 text-xs cursor-help"
                    :title="ver.deprecation_message ?? t('packageDetailPage.deprecated')"
                  >
                    deprecated
                  </Badge>
                  <Badge v-if="ver.unlisted" variant="secondary" class="ml-1 text-xs">
                    unlisted
                  </Badge>
                  <!-- Under the version rather than in a column of its own: the
                       licence is an attribute of this version, and the table is
                       already seven columns wide.

                       A stated "unknown" rather than a blank when null —
                       rendering nothing would make "no manifest parser for this
                       registry type" indistinguishable from "declares no
                       licence" (RFC 0004-bis §13.1). -->
                  <p
                    class="text-xs text-muted-foreground truncate max-w-[200px]"
                    :title="ver.license ?? t('packageDetailPage.licenseUnknownHelp')"
                  >
                    {{ ver.license ?? t("packageDetailPage.licenseUnknown") }}
                  </p>
                </TableCell>
                <TableCell>
                  <!-- Three values, not two. `upstream` means this instance
                       holds no bytes for the version and knows about it only
                       because it asked — the badge says so rather than letting
                       it read as something we have. -->
                  <Badge
                    :variant="
                      ver.source === 'local'
                        ? 'secondary'
                        : isUpstreamOnly(ver.source)
                          ? 'outline'
                          : 'outline'
                    "
                    class="text-xs"
                    :class="isUpstreamOnly(ver.source) ? 'border-dashed' : ''"
                    :title="
                      isUpstreamOnly(ver.source)
                        ? t('packageDetailPage.notHeldHereHelp')
                        : undefined
                    "
                  >
                    {{
                      ver.source === "local"
                        ? t("packageDetailPage.local")
                        : isUpstreamOnly(ver.source)
                          ? t("packageDetailPage.notHeldHere")
                          : t("common.proxied")
                    }}
                  </Badge>
                  <!-- The door beside the wall. RFC 0007 made this row honest
                       about not holding the version; this is what a reader can
                       do about it, and it sits next to the mark that told them
                       (RFC 0007-bis §2.3, §4.4). -->
                  <template v-if="isUpstreamOnly(ver.source)">
                    <Button
                      v-if="data?.fetch.offered"
                      size="sm"
                      variant="outline"
                      class="ml-2 h-6 px-2 text-xs"
                      :disabled="fetching !== null"
                      :title="t('packageDetailPage.fetchVersionTitle')"
                      @click="onFetchVersion(ver.version)"
                    >
                      {{
                        fetching === ver.version
                          ? t("packageDetailPage.fetching")
                          : t("packageDetailPage.fetchVersion")
                      }}
                    </Button>
                    <!-- Not a disabled button with no explanation: where "fetch
                         this version" has no single meaning, the kind's own
                         reason is shown instead — the same string the endpoint
                         and the support table use. -->
                    <span
                      v-else-if="data?.fetch.reason"
                      class="ml-2 text-xs text-muted-foreground"
                      >{{
                        t("packageDetailPage.fetchUnavailable", {
                          reason: data.fetch.reason,
                        })
                      }}</span
                    >
                    <!-- A pull is an authenticated act: it fills this instance's
                         cache, spends bandwidth and writes an audit row, so
                         `explore_fetch_version` answers `401` without a session
                         and the server stops offering the button (RFC 0007-bis
                         §4.1, revisited).

                         Said, not merely withheld. The absent button is the same
                         absence as "this registry kind cannot do it", and the two
                         are a sign-in away from each other — so the reader is
                         told which, in the console's own translated words. The
                         server sends no reason for this one because *whether
                         there is a session* is the one half of the question the
                         page knows better than the endpoint does. -->
                    <RouterLink
                      v-else-if="!isAuthenticated"
                      :to="{ path: '/login', query: { redirect: route.fullPath } }"
                      class="ml-2 text-xs text-muted-foreground underline underline-offset-[3px] hover:text-foreground"
                      @click.stop
                      >{{ t("packageDetailPage.fetchNeedsSession") }}</RouterLink
                    >
                    <span
                      v-if="fetchResult[ver.version]"
                      class="ml-2 text-xs text-muted-foreground"
                      >{{ fetchResult[ver.version] }}</span
                    >
                  </template>
                </TableCell>
                <TableCell>
                  <RouterLink
                    v-if="ver.firewall.status === 'blocked'"
                    :to="{
                      path: '/tools/access-check',
                      query: { registry, name, version: ver.version },
                    }"
                    class="mr-2 font-mono text-xs underline underline-offset-4 text-muted-foreground hover:text-foreground"
                    >{{ t("packageDetailPage.why") }}</RouterLink
                  >
                  <span v-if="ver.firewall.status === 'blocked'" class="group relative">
                    <Badge variant="destructive" class="text-xs cursor-help">{{
                      t("common.blocked")
                    }}</Badge>
                    <span
                      class="absolute bottom-full left-0 mb-1 hidden group-hover:block z-10 w-64 rounded-sm bg-popover border p-2 text-xs text-popover-foreground shadow-md"
                    >
                      <strong>{{ t("common.reasonLabel") }}</strong>
                      {{ (ver.firewall as any).reason }}<br />
                      <strong>By:</strong> {{ (ver.firewall as any).blocked_by }}<br />
                      <strong>At:</strong>
                      {{ formatDate((ver.firewall as any).blocked_at) }}
                    </span>
                  </span>
                  <!-- Resolution as state (DESIGN.md; RFC 0004-bis §7 item 7).
                       The blocked branch above keeps its crimson badge and its
                       hover note: a refusal has to state its rule, which is the
                       denial note's job, not this mark's. -->
                  <Resolution
                    v-else
                    :state="firewallResolution(ver.firewall)"
                    :label="firewallLabel(ver.firewall)"
                  />
                </TableCell>
                <!-- `unknown`, never `0`: a definite-looking number for a
                     version this instance has never held would be a claim we
                     cannot support (RFC 0007 §4.2). -->
                <TableCell class="text-right text-sm text-muted-foreground">
                  {{
                    ver.download_count === null || ver.download_count === undefined
                      ? t("common.unknown")
                      : formatCount(ver.download_count)
                  }}
                </TableCell>
                <TableCell class="text-sm text-muted-foreground">
                  {{
                    isUpstreamOnly(ver.source)
                      ? t("common.unknown")
                      : formatDate(ver.last_accessed ?? null)
                  }}
                </TableCell>
                <TableCell class="text-sm text-muted-foreground">
                  {{ formatDate(ver.published_at ?? null) }}
                </TableCell>
                <TableCell class="text-sm">
                  <div class="flex flex-wrap items-center gap-1">
                    <span
                      v-for="vuln in ver.vulnerabilities"
                      :key="vuln.osv_id"
                      class="group relative"
                    >
                      <Badge :variant="severityVariant(vuln.severity)" class="text-xs cursor-help">
                        {{ vuln.severity }}
                      </Badge>
                      <span
                        class="absolute bottom-full left-0 mb-1 hidden group-hover:block z-10 w-64 rounded-sm bg-popover border p-2 text-xs text-popover-foreground shadow-md"
                      >
                        <strong>{{ vuln.osv_id }}</strong
                        ><br />
                        {{ vuln.summary }}
                        <template v-if="vuln.fixed_version">
                          <br /><strong>{{ t("packageDetailPage.fixedIn") }}</strong>
                          {{ vuln.fixed_version }}
                        </template>
                      </span>
                    </span>
                    <a
                      v-if="ver.socket_badge_url"
                      :href="ver.socket_badge_url"
                      target="_blank"
                      rel="noopener noreferrer"
                      :title="t('packageDetailPage.supplyChainReportOn')"
                    >
                      <img :src="ver.socket_badge_url" alt="socket.dev" class="h-4" />
                    </a>
                    <!-- An empty list means *scanned and clear* only when
                         something has scanned it. On a version nothing has ever
                         opened it means *never scanned*, and the two must not
                         render identically. -->
                    <span
                      v-if="!ver.vulnerabilities_scanned"
                      class="text-muted-foreground text-xs"
                      :title="t('packageDetailPage.notScannedHelp')"
                    >
                      {{ t("packageDetailPage.notScanned") }}
                    </span>
                    <span
                      v-else-if="ver.vulnerabilities.length === 0 && !ver.socket_badge_url"
                      class="text-muted-foreground text-xs"
                    >
                      —
                    </span>
                  </div>
                </TableCell>
                <TableCell v-if="token" class="text-sm">
                  <span
                    v-if="sbomMissing.has(`${registry}/${name}/${ver.version}`)"
                    class="text-muted-foreground text-xs"
                    >{{ t("packageDetailPage.noSbom") }}</span
                  >
                  <div v-else class="flex gap-1">
                    <button
                      :disabled="sbomLoading === `${registry}/${name}/${ver.version}:spdx`"
                      class="inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-xs hover:bg-accent disabled:opacity-50"
                      :title="t('packageDetailPage.downloadSpdx23')"
                      @click="downloadSbom(ver.version, 'spdx')"
                    >
                      <FileJson class="h-3 w-3" />
                      SPDX
                    </button>
                    <button
                      :disabled="sbomLoading === `${registry}/${name}/${ver.version}:cyclonedx`"
                      class="inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-xs hover:bg-accent disabled:opacity-50"
                      :title="t('packageDetailPage.downloadCyclonedx14')"
                      @click="downloadSbom(ver.version, 'cyclonedx')"
                    >
                      <FileCode class="h-3 w-3" />
                      CDX
                    </button>
                  </div>
                </TableCell>
                <!-- Download link -->
                <TableCell class="text-sm">
                  <a
                    v-if="downloadUrl(ver.version)"
                    :href="downloadUrl(ver.version)!"
                    target="_blank"
                    rel="noopener noreferrer"
                    class="inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-xs hover:bg-accent"
                    :title="
                      t('packageDetailPage.downloadViaProxy', {
                        version: ver.version,
                      })
                    "
                  >
                    <Download class="h-3 w-3" />
                    {{ t("common.download") }}
                  </a>
                  <span v-else class="text-muted-foreground text-xs">—</span>
                </TableCell>
              </TableRow>
              <!-- A filter that matched nothing is not a package with no
                   versions, and answering the first with the second's words
                   ("nothing has been pulled through") would describe the
                   instance to a reader who asked about their own typing. The
                   counter above already says `0 of 60 shown`; this says which
                   of the two absences the empty table is. -->
              <TableRow v-if="pagedVersions.length === 0 && unfilteredTotal > 0">
                <TableCell :colspan="token ? 9 : 8" class="text-center text-muted-foreground py-6">
                  <EmptyState
                    :title="t('packageDetailPage.noVersionsMatch')"
                    :description="t('packageDetailPage.noVersionsMatchHint')"
                  />
                </TableCell>
              </TableRow>
              <TableRow v-else-if="pagedVersions.length === 0">
                <TableCell :colspan="token ? 9 : 8" class="text-center text-muted-foreground py-6">
                  <!-- Two different absences, and the reader needs to know
                       which: "nothing has been pulled through, and the upstream
                       does not have it either" is an answer; "the upstream
                       could not be reached" is a gap. -->
                  <EmptyState
                    :title="t('packageDetailPage.noVersionsYet')"
                    :description="
                      data.upstream.error
                        ? t('packageDetailPage.upstreamUnreachable')
                        : data.upstream.attempted
                          ? t('packageDetailPage.upstreamHasNoneEither')
                          : t('packageDetailPage.nothingHasBeenPulledThrough')
                    "
                  />
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
          <div v-if="versionTotalPages > 1" class="border-t border-border px-4 py-3">
            <Pagination
              :page="versionPage"
              :total-pages="versionTotalPages"
              @update:page="turnToPage($event)"
            />
          </div>
        </CardContent>
      </Card>
    </template>
  </div>

  <!-- ── Administration ──────────────────────────────────────────────────
         Everything AdminPackageDetail used to be, in place. -->
  <template v-if="isAdmin">
    <Separator class="my-6" />
    <section class="space-y-4" aria-labelledby="admin-heading">
      <h2
        id="admin-heading"
        class="font-mono text-sm font-semibold uppercase tracking-wider text-copper"
      >
        {{ t("common.administration") }}
      </h2>

      <p v-if="adminError" class="text-sm text-destructive">{{ adminError }}</p>

      <template v-else-if="adminData">
        <PackageVersionsTable
          :registry="registry"
          :name="name"
          :versions="adminData.versions"
          @reload="reloadAdmin"
        />
        <PackageBetaChannel :registry="registry" />
        <PackageVisibility :registry="registry" :name="name" />
        <PackageEventsTable :events="adminData.recent_events" />
      </template>
    </section>
  </template>
</template>
