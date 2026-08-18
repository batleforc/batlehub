<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { ref, computed, onMounted } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";
import {
  ArrowLeft,
  ShieldCheck,
  ShieldAlert,
  Lock,
  Unlock,
  Package,
  FileJson,
  FileCode,
  Download,
} from "@lucide/vue";
import { explorePackageDetail, listRegistries } from "@/client/sdk.gen";
import type {
  ExplorePackageDetailResponse,
  FirewallDto,
  RegistryInfo,
} from "@/client/types.gen";
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
import { formatCount } from "@/lib/format";
import { severityVariant } from "@/lib/badge-variants";
import { Badge } from "@/components/ui/badge";
import { Resolution, type ResolutionState } from "@/components/ui/resolution";
import { EmptyState } from "@/components/ui/empty-state";
import { Button } from "@/components/ui/button";
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

const { token, isAdmin } = useAuth();
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
  () =>
    registriesList.value?.find((r) => r.name === registry.value)?.type ?? null,
);

const data = ref<ExplorePackageDetailResponse | null>(null);
const loading = ref(false);
const error = ref<string | null>(null);

/**
 * The version the README panel follows.
 *
 * `null` until the reader picks one, which asks the endpoint for the newest
 * version that has a README — the same thing the page would have chosen, and
 * one fewer assumption about ordering duplicated on the client.
 */
const selectedVersion = ref<string | null>(null);

/** Upstream-only rows, for the notice's count. */
const upstreamVersionCount = computed(
  () => data.value?.versions.filter((v) => v.source === "upstream").length ?? 0,
);

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

async function fetchDetail() {
  loading.value = true;
  error.value = null;
  try {
    const { data: res, error: apiErr } = await explorePackageDetail({
      path: { registry: registry.value, name: name.value },
    });
    if (apiErr) throw new Error(`HTTP error`);
    data.value = res as ExplorePackageDetailResponse;
    // The page's initial selection is unchanged by RFC 0007: the first row of
    // the existing sort, which is stable-before-pre-release, newest first.
    selectedVersion.value = data.value.versions[0]?.version ?? null;
  } catch (e) {
    error.value = extractMessage(e);
  } finally {
    loading.value = false;
  }
}

function goBack() {
  router.push({
    path: "/packages",
    query: { registry: registry.value },
  });
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

onMounted(fetchDetail);

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
  <div class="space-y-6 max-w-4xl">
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
      <!-- Header -->
      <div class="flex items-start gap-3 flex-wrap">
        <div class="flex-1">
          <div class="flex items-center gap-2 flex-wrap">
            <Package class="h-6 w-6 text-primary shrink-0" />
            <h1 class="text-2xl font-bold font-mono">{{ data.name }}</h1>
            <Badge variant="outline">{{ data.registry }}</Badge>
          </div>
          <p class="text-sm text-muted-foreground mt-1">
            {{ t("packageDetailPage.knownVersions", data.versions.length) }}
          </p>
        </div>
        <Button variant="outline" size="sm" @click="fetchDetail">
          {{ t("common.refresh") }}
        </Button>
      </div>

      <!-- Gate summary card -->
      <Card>
        <CardHeader class="pb-2">
          <CardTitle class="text-base">{{
            t("packageDetailPage.accessGate")
          }}</CardTitle>
        </CardHeader>
        <CardContent>
          <div class="space-y-2">
            <!-- Registry access -->
            <div class="flex items-center gap-2 text-sm">
              <component
                :is="data.gate.registry_accessible ? ShieldCheck : ShieldAlert"
                :class="
                  data.gate.registry_accessible
                    ? 'text-primary'
                    : 'text-destructive'
                "
                class="h-4 w-4 shrink-0"
              />
              <span class="text-muted-foreground">{{
                t("packageDetailPage.registryAccess")
              }}</span>
              <span
                :class="
                  data.gate.registry_accessible
                    ? 'text-primary font-medium'
                    : 'text-destructive font-medium'
                "
              >
                {{
                  data.gate.registry_accessible
                    ? t("accessCheck.allowed")
                    : t("accessCheck.denied")
                }}
              </span>
            </div>

            <!-- Beta channel -->
            <div class="flex items-center gap-2 text-sm">
              <component
                :is="data.gate.beta_member ? Unlock : Lock"
                :class="
                  data.gate.beta_member
                    ? 'text-primary'
                    : 'text-muted-foreground'
                "
                class="h-4 w-4 shrink-0"
              />
              <span class="text-muted-foreground">{{
                t("packageDetailPage.betaChannel")
              }}</span>
              <span
                :class="
                  data.gate.beta_member
                    ? 'text-primary font-medium'
                    : 'text-muted-foreground'
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
      <ReadmePanel
        :registry="registry"
        :name="name"
        :version="selectedVersion"
      />

      <!-- Versions table -->
      <Card>
        <CardHeader class="pb-2">
          <CardTitle class="text-base">{{ t("common.versions") }}</CardTitle>
        </CardHeader>
        <CardContent class="p-0">
          <div v-if="data.upstream.attempted" class="px-4 pb-2">
            <UpstreamNotice
              :upstream="data.upstream"
              :upstream-version-count="upstreamVersionCount"
            />
          </div>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{{ t("common.version") }}</TableHead>
                <TableHead>{{ t("common.source") }}</TableHead>
                <TableHead>{{ t("common.firewall") }}</TableHead>
                <TableHead class="text-right">{{
                  t("common.downloads")
                }}</TableHead>
                <TableHead>{{ t("packageDetailPage.lastAccessed") }}</TableHead>
                <TableHead>{{ t("common.published") }}</TableHead>
                <TableHead>{{ t("common.security") }}</TableHead>
                <TableHead v-if="token">SBOM</TableHead>
                <TableHead>{{ t("common.download") }}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow
                v-for="ver in data.versions"
                :key="`${ver.version}-${ver.source}`"
                :class="[
                  ver.is_prerelease ? 'text-muted-foreground italic' : '',
                  selectedVersion === ver.version ? 'bg-muted/40' : '',
                  'cursor-pointer',
                ]"
                @click="selectedVersion = ver.version"
              >
                <TableCell class="font-mono text-sm">
                  {{ ver.version }}
                  <Badge
                    v-if="ver.is_prerelease"
                    variant="outline"
                    class="ml-1 text-xs"
                  >
                    pre-release
                  </Badge>
                  <Badge
                    v-if="ver.deprecated"
                    variant="destructive"
                    class="ml-1 text-xs cursor-help"
                    :title="
                      ver.deprecation_message ??
                      t('packageDetailPage.deprecated')
                    "
                  >
                    deprecated
                  </Badge>
                  <Badge
                    v-if="ver.unlisted"
                    variant="secondary"
                    class="ml-1 text-xs"
                  >
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
                    :title="
                      ver.license ?? t('packageDetailPage.licenseUnknownHelp')
                    "
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
                  <span
                    v-if="ver.firewall.status === 'blocked'"
                    class="group relative"
                  >
                    <Badge variant="destructive" class="text-xs cursor-help">{{
                      t("common.blocked")
                    }}</Badge>
                    <span
                      class="absolute bottom-full left-0 mb-1 hidden group-hover:block z-10 w-64 rounded-sm bg-popover border p-2 text-xs text-popover-foreground shadow-md"
                    >
                      <strong>{{ t("common.reasonLabel") }}</strong>
                      {{ (ver.firewall as any).reason }}<br />
                      <strong>By:</strong> {{ (ver.firewall as any).blocked_by
                      }}<br />
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
                    ver.download_count === null ||
                    ver.download_count === undefined
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
                      <Badge
                        :variant="severityVariant(vuln.severity)"
                        class="text-xs cursor-help"
                      >
                        {{ vuln.severity }}
                      </Badge>
                      <span
                        class="absolute bottom-full left-0 mb-1 hidden group-hover:block z-10 w-64 rounded-sm bg-popover border p-2 text-xs text-popover-foreground shadow-md"
                      >
                        <strong>{{ vuln.osv_id }}</strong
                        ><br />
                        {{ vuln.summary }}
                        <template v-if="vuln.fixed_version">
                          <br /><strong>{{
                            t("packageDetailPage.fixedIn")
                          }}</strong>
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
                      <img
                        :src="ver.socket_badge_url"
                        alt="socket.dev"
                        class="h-4"
                      />
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
                      v-else-if="
                        ver.vulnerabilities.length === 0 &&
                        !ver.socket_badge_url
                      "
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
                      :disabled="
                        sbomLoading ===
                        `${registry}/${name}/${ver.version}:spdx`
                      "
                      class="inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-xs hover:bg-accent disabled:opacity-50"
                      :title="t('packageDetailPage.downloadSpdx23')"
                      @click="downloadSbom(ver.version, 'spdx')"
                    >
                      <FileJson class="h-3 w-3" />
                      SPDX
                    </button>
                    <button
                      :disabled="
                        sbomLoading ===
                        `${registry}/${name}/${ver.version}:cyclonedx`
                      "
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
              <TableRow v-if="data.versions.length === 0">
                <TableCell
                  :colspan="token ? 9 : 8"
                  class="text-center text-muted-foreground py-6"
                >
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
