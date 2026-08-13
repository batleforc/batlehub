<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { ref, computed } from "vue";
import { useRouter } from "vue-router";
import { Package } from "@lucide/vue";
import SectionTabs from "@/components/admin/SectionTabs.vue";
import { PageHeader } from "@/components/ui/page-header";
import { PACKAGES_TABS } from "@/config/adminSections";
import {
  listPackages,
  listRegistries,
  blockPackage,
  unblockPackage,
  bulkBlockPackages,
  bulkUnblockPackages,
} from "@/client/sdk.gen";
import type { RegistryInfo } from "@/client/types.gen";
import { useApi, extractMessage } from "@/composables/useApi";
import { DestructiveConfirm } from "@/components/ui/destructive-confirm";
import { useAuth } from "@/composables/useAuth";
import { useAuthFetch } from "@/composables/useAuthFetch";
import { formatDate as fmtDate } from "@/lib/format";
import { API_BASE_URL } from "@/config";
import { AsyncState } from "@/components/ui/async-state";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardHeader, CardContent } from "@/components/ui/card";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";

const { t } = useI18n();

interface AdminPackageSummary {
  id: string;
  package_id: {
    registry: string;
    name: string;
    version: string;
    artifact?: string | null;
  };
  status:
    | { status: "available" }
    | {
        status: "blocked";
        reason: string;
        blocked_by: string;
        blocked_at: string;
      };
  last_accessed: string | null;
  last_accessed_by: string | null;
  access_count: number;
}

const { token } = useAuth();
const { authFetch } = useAuthFetch();
const router = useRouter();

interface AdminPackageListResponse {
  items: AdminPackageSummary[];
  total: number;
  page: number;
  per_page: number;
}

/**
 * Server-side filters, not a 1000-row fetch filtered in the browser.
 *
 * `GET /api/v1/admin/packages` declares `registry`, `name`, `blocked_only`,
 * `page` and `per_page`; the page sent only `per_page: 1000`. So "what is
 * blocked right now" — half of this page's question — was answerable by the
 * API and unreachable in the UI, and the header count silently capped at a
 * thousand.
 */
const registryFilter = ref("");
const blockedOnly = ref(false);

const {
  data: packagesResponse,
  error,
  loading,
  reload,
} = useApi<AdminPackageListResponse>(
  () =>
    listPackages({
      query: {
        per_page: 1000,
        ...(registryFilter.value ? { registry: registryFilter.value } : {}),
        ...(blockedOnly.value ? { blocked_only: true } : {}),
      },
    }) as Promise<{
      data?: unknown;
      error?: unknown;
    }>,
  [token, registryFilter, blockedOnly],
);

const packages = computed(() => packagesResponse.value?.items ?? null);

const actionError = ref<string | null>(null);

const { data: registries } = useApi<RegistryInfo[]>(
  () => listRegistries() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

const search = ref("");

const filteredPackages = computed(() => {
  if (!packages.value) return [];
  const q = search.value.toLowerCase().trim();
  if (!q) return packages.value;
  return packages.value.filter(
    (p) =>
      p.package_id.name.toLowerCase().includes(q) ||
      p.package_id.registry.toLowerCase().includes(q) ||
      p.package_id.version.toLowerCase().includes(q),
  );
});

// ── Block existing package ────────────────────────────────────────────────────

async function block(pkg: AdminPackageSummary) {
  const reason = blockReason.value.trim();
  if (!reason) return;
  actionError.value = null;
  try {
    await blockPackage({
      body: {
        registry: pkg.package_id.registry,
        name: pkg.package_id.name,
        version: pkg.package_id.version,
        artifact: pkg.package_id.artifact,
        reason,
      },
    });
    reload();
  } catch (e: unknown) {
    actionError.value = extractMessage(e);
  }
}

async function unblock(pkg: AdminPackageSummary) {
  actionError.value = null;
  try {
    await unblockPackage({
      body: {
        registry: pkg.package_id.registry,
        name: pkg.package_id.name,
        version: pkg.package_id.version,
        artifact: pkg.package_id.artifact,
      },
    });
    reload();
  } catch (e: unknown) {
    actionError.value = extractMessage(e);
  }
}

async function deletePkg(pkg: AdminPackageSummary) {
  actionError.value = null;
  try {
    const res = await authFetch(`${API_BASE_URL}/api/v1/admin/packages/delete`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        registry: pkg.package_id.registry,
        name: pkg.package_id.name,
        version: pkg.package_id.version,
        artifact: pkg.package_id.artifact ?? null,
      }),
    });
    if (!res.ok) {
      const body = (await res.json().catch(() => ({}))) as { error?: string };
      throw new Error(body.error ?? `HTTP ${res.status}`);
    }
    reload();
  } catch (e: unknown) {
    actionError.value = extractMessage(e);
  }
}

// ── Multi-select + bulk actions ───────────────────────────────────────────────

const selected = ref<Set<string>>(new Set());
const bulkLoading = ref(false);
const bulkResultMsg = ref<string | null>(null);

function pkgKey(pkg: AdminPackageSummary) {
  return `${pkg.package_id.registry}:${pkg.package_id.name}:${pkg.package_id.version}:${pkg.package_id.artifact ?? ""}`;
}

const allSelected = computed(
  () =>
    filteredPackages.value.length > 0 &&
    filteredPackages.value.every((p) => selected.value.has(pkgKey(p))),
);

function toggleAll() {
  if (allSelected.value) {
    filteredPackages.value.forEach((p) => selected.value.delete(pkgKey(p)));
  } else {
    filteredPackages.value.forEach((p) => selected.value.add(pkgKey(p)));
  }
  selected.value = new Set(selected.value);
}

function toggleOne(pkg: AdminPackageSummary) {
  const k = pkgKey(pkg);
  if (selected.value.has(k)) selected.value.delete(k);
  else selected.value.add(k);
  selected.value = new Set(selected.value);
}

const selectedPackages = computed(() =>
  (packages.value ?? []).filter((p) => selected.value.has(pkgKey(p))),
);

async function bulkBlock() {
  const reason = blockReason.value.trim();
  if (!reason) return;
  bulkLoading.value = true;
  bulkResultMsg.value = null;
  try {
    const res = await bulkBlockPackages({
      body: {
        items: selectedPackages.value.map((p) => ({
          registry: p.package_id.registry,
          name: p.package_id.name,
          version: p.package_id.version,
          artifact: p.package_id.artifact ?? null,
          reason,
        })),
      },
    });
    if (res.error) {
      // Without this the request failed, the selection cleared, the table
      // reloaded, and the page said nothing at all — a failed bulk action
      // looked exactly like a successful one.
      actionError.value = extractMessage(res.error);
      return;
    }
    const r = res.data;
    if (r) {
      const failSuffix = r.failed_count ? `, ${r.failed_count} failed` : "";
      bulkResultMsg.value = `Blocked ${r.succeeded_count} package(s)${failSuffix}.`;
    }
  } finally {
    bulkLoading.value = false;
    selected.value = new Set();
    reload();
  }
}

async function bulkUnblock() {
  bulkLoading.value = true;
  bulkResultMsg.value = null;
  try {
    const res = await bulkUnblockPackages({
      body: {
        items: selectedPackages.value.map((p) => ({
          registry: p.package_id.registry,
          name: p.package_id.name,
          version: p.package_id.version,
          artifact: p.package_id.artifact ?? null,
        })),
      },
    });
    if (res.error) {
      // Without this the request failed, the selection cleared, the table
      // reloaded, and the page said nothing at all — a failed bulk action
      // looked exactly like a successful one.
      actionError.value = extractMessage(res.error);
      return;
    }
    const r = res.data;
    if (r) {
      const failSuffix = r.failed_count ? `, ${r.failed_count} failed` : "";
      bulkResultMsg.value = `Unblocked ${r.succeeded_count} package(s)${failSuffix}.`;
    }
  } finally {
    bulkLoading.value = false;
    selected.value = new Set();
    reload();
  }
}

async function bulkDelete() {
  bulkLoading.value = true;
  bulkResultMsg.value = null;
  try {
    const res = await authFetch(`${API_BASE_URL}/api/v1/admin/packages/bulk-delete`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        items: selectedPackages.value.map((p) => ({
          registry: p.package_id.registry,
          name: p.package_id.name,
          version: p.package_id.version,
          artifact: p.package_id.artifact ?? null,
        })),
      }),
    });
    const json = (await res.json().catch(() => ({}))) as {
      succeeded_count?: number;
      failed_count?: number;
    };
    const failSuffix = json.failed_count ? `, ${json.failed_count} failed` : "";
    bulkResultMsg.value = `Deleted ${json.succeeded_count ?? 0} package(s)${failSuffix}.`;
  } catch (e) {
    bulkResultMsg.value = extractMessage(e);
  } finally {
    bulkLoading.value = false;
    selected.value = new Set();
    reload();
  }
}

// ── Pre-block form ────────────────────────────────────────────────────────────

/**
 * The "Block a package" form is gone (RFC 0004 Phase 5, *distill*).
 *
 * It asked an operator to type a registry, name, version and artifact by hand
 * — while a list of packages sat on the same screen — and it was a one-row
 * copy of `/admin/packages/bulk`, one tab away, which does the same job with
 * validation, a preview table and a per-item failure report this page threw
 * away. Blocking something that is *not here yet* is that page's question; the
 * toolbar links to it rather than answering it twice.
 */

type PendingAction =
  | { kind: "delete-one"; pkg: AdminPackageSummary }
  | { kind: "block-one"; pkg: AdminPackageSummary }
  | { kind: "bulk-block" }
  | { kind: "bulk-delete" }
  | { kind: "bulk-unblock" };

/** The reason a block carries, collected by the dialog rather than a prompt. */
const blockReason = ref("");

const pending = ref<PendingAction | null>(null);

const confirmProps = computed(() => {
  const action = pending.value;
  if (!action) return null;
  if (action.kind === "delete-one") {
    const id = action.pkg.package_id;
    return {
      action: "Delete",
      count: 1,
      itemNoun: "package record",
      scope: `${id.name}@${id.version} in ${id.registry}, and its cached artifact`,
      reversible: false,
      confirmName: id.name,
    };
  }
  if (action.kind === "block-one") {
    const id = action.pkg.package_id;
    return {
      action: "Block",
      count: 1,
      itemNoun: "package",
      scope: `${id.name}@${id.version} in ${id.registry} — consumers resolving it receive 403 until unblocked`,
      reversible: true,
    };
  }
  if (action.kind === "bulk-block") {
    return {
      action: "Block",
      count: selected.value.size,
      itemNoun: "package",
      scope: "the selection — consumers resolving them receive 403 until unblocked",
      reversible: true,
    };
  }
  if (action.kind === "bulk-delete") {
    return {
      action: "Delete",
      count: selected.value.size,
      itemNoun: "package record",
      scope: "the selection, purging their cached artifacts",
      reversible: false,
      confirmName: "delete",
    };
  }
  return {
    action: "Unblock",
    count: selected.value.size,
    itemNoun: "package",
    scope: "the selection",
    reversible: true,
  };
});

async function runPending(): Promise<void> {
  const action = pending.value;
  pending.value = null;
  if (!action) return;
  if (action.kind === "delete-one") await deletePkg(action.pkg);
  else if (action.kind === "block-one") await block(action.pkg);
  else if (action.kind === "bulk-block") await bulkBlock();
  else if (action.kind === "bulk-delete") await bulkDelete();
  else await bulkUnblock();
}
</script>

<template>
  <div class="space-y-4">
    <SectionTabs :tabs="PACKAGES_TABS" />
    <PageHeader variant="display">
      <template #title>
        {{ t("adminNav.allPackages") }}
        <span v-if="packages?.length" class="font-mono text-base font-normal text-muted-foreground"
          >({{ packages.length }})</span
        >
      </template>
    </PageHeader>
    <!-- The form that used to sit here is `/admin/packages/bulk`, one tab
         away and better at the job. A link, not a second implementation. -->
    <p class="text-sm text-muted-foreground">
      <i18n-t keypath="adminPackages.blockNotHereYet" tag="span">
        <template #link>
          <RouterLink to="/admin/packages/bulk" class="text-primary hover:underline">{{
            t("adminNav.bulkBlock")
          }}</RouterLink>
        </template>
      </i18n-t>
    </p>

    <!-- Bulk action bar -->
    <div
      v-if="selected.size > 0"
      class="sticky top-16 z-30 flex items-center gap-3 rounded-sm border bg-card px-4 py-2.5 shadow-sm"
    >
      <span class="text-sm font-medium">{{ selected.size }} selected</span>
      <Button
        size="sm"
        variant="destructive"
        :disabled="bulkLoading"
        @click="
          blockReason = '';
          pending = { kind: 'bulk-block' };
        "
        >{{
        t("adminPackages.blockSelected")
      }}</Button>
      <Button
        size="sm"
        variant="outline"
        :disabled="bulkLoading"
        @click="pending = { kind: 'bulk-unblock' }"
        >{{ t("adminPackages.unblockSelected") }}</Button
      >
      <Button
        size="sm"
        variant="destructive"
        :disabled="bulkLoading"
        @click="pending = { kind: 'bulk-delete' }"
        >{{ t("adminPackages.deleteSelected") }}</Button
      >
      <Button size="sm" variant="ghost" @click="selected = new Set()">
        {{ t("common.clearAction") }}
      </Button>
    </div>

    <!-- Outside the selection-gated bar above: every bulk handler clears the
         selection in its `finally`, so a message rendered inside that bar was
         written into a node destroyed in the same tick and never seen. -->
    <p v-if="bulkResultMsg" role="status" class="text-sm text-muted-foreground">
      {{ bulkResultMsg }}
    </p>

    <!-- Package list -->
    <Card>
      <CardHeader class="space-y-3 pb-3">
        <div class="flex flex-row items-center justify-end space-y-0">
          <Button variant="outline" size="sm" @click="reload"> {{ t("common.refresh") }} </Button>
        </div>
        <div class="flex flex-wrap items-center gap-2">
          <Input
            v-model="search"
            :placeholder="t('adminPackages.filterByNameRegistry')"
            :aria-label="t('adminPackages.filterPackages')"
            class="max-w-sm h-8 text-sm"
          />
          <!-- Both go to the server. `blocked_only` answers half this page's
               question and had no control at all. -->
          <select
            v-model="registryFilter"
            :aria-label="t('common.registry')"
            class="h-8 rounded-sm border border-input bg-transparent px-2 text-sm"
          >
            <option value="">{{ t("adminPackages.allRegistries") }}</option>
            <option v-for="r in registries ?? []" :key="r.name" :value="r.name">
              {{ r.name }}
            </option>
          </select>
          <label class="flex items-center gap-1.5 text-sm text-muted-foreground">
            <input v-model="blockedOnly" type="checkbox" class="h-3.5 w-3.5" />
            {{ t("adminPackages.blockedOnly") }}
          </label>
        </div>
      </CardHeader>
      <CardContent class="p-0">
        <p v-if="actionError" class="px-6 pt-4 text-sm text-destructive">
          {{ actionError }}
        </p>
        <AsyncState :loading="loading" :error="error" :empty="!filteredPackages.length">
          <template #empty>
            <div class="py-12 text-center space-y-2">
              <Package class="h-8 w-8 mx-auto text-muted-foreground/50" />
              <p class="text-sm text-muted-foreground">
                {{
                  search
                    ? t("adminPackages.noPackagesMatchYourFilter")
                    : t("adminPackages.noPackagesYet")
                }}
              </p>
              <p v-if="search" class="text-xs text-muted-foreground">
                {{ t("adminPackages.tryClearingTheFilter") }}
              </p>
            </div>
          </template>

          <Table>
            <TableHeader>
              <TableRow>
                <TableHead class="w-8">
                  <input
                    type="checkbox"
                    :aria-label="t('adminPackages.selectAllPackages')"
                    :checked="allSelected"
                    class="cursor-pointer"
                    @change="toggleAll"
                  />
                </TableHead>
                <TableHead>{{ t("common.registry") }}</TableHead>
                <TableHead>{{ t("common.name") }}</TableHead>
                <TableHead>{{ t("common.version") }}</TableHead>
                <TableHead>{{ t("common.artifact") }}</TableHead>
                <TableHead>{{ t("common.status") }}</TableHead>
                <TableHead>{{ t("adminPackages.lastPulled") }}</TableHead>
                <TableHead>{{ t("adminPackages.lastPulledBy") }}</TableHead>
                <TableHead class="text-right"> {{ t("common.downloads") }} </TableHead>
                <TableHead />
                <TableHead class="text-right"> {{ t("common.actions") }} </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow
                v-for="(pkg, i) in filteredPackages"
                :key="i"
                :class="pkg.status.status === 'blocked' ? 'bg-destructive/5' : ''"
              >
                <TableCell class="w-8">
                  <input
                    type="checkbox"
                    :aria-label="t('adminPackages.selectPackage', { name: pkg.package_id.name })"
                    :checked="selected.has(pkgKey(pkg))"
                    class="cursor-pointer"
                    @change="toggleOne(pkg)"
                  />
                </TableCell>
                <TableCell class="font-mono text-xs">
                  {{ pkg.package_id.registry }}
                </TableCell>
                <TableCell class="font-medium">
                  {{ pkg.package_id.name }}
                </TableCell>
                <TableCell class="font-mono text-xs">
                  {{ pkg.package_id.version }}
                </TableCell>
                <TableCell class="text-muted-foreground text-xs font-mono">
                  {{ pkg.package_id.artifact ?? "—" }}
                </TableCell>
                <TableCell>
                  <div class="space-y-0.5">
                    <Badge :variant="pkg.status.status === 'blocked' ? 'destructive' : 'secondary'">
                      {{
                        pkg.status.status === "blocked"
                          ? t("common.blocked")
                          : t("packageVersionsTable.available")
                      }}
                    </Badge>
                    <p v-if="pkg.status.status === 'blocked'" class="text-xs text-muted-foreground">
                      {{ pkg.status.reason }}
                    </p>
                  </div>
                </TableCell>
                <TableCell class="text-xs tabular-nums whitespace-nowrap">
                  {{ fmtDate(pkg.last_accessed) }}
                </TableCell>
                <TableCell class="text-sm">
                  <span v-if="pkg.last_accessed_by" class="font-medium">{{
                    pkg.last_accessed_by
                  }}</span>
                  <span v-else-if="pkg.access_count > 0" class="text-muted-foreground italic"
                    >anonymous</span
                  >
                  <span v-else class="text-muted-foreground">—</span>
                </TableCell>
                <TableCell class="text-right tabular-nums">
                  {{ pkg.access_count }}
                </TableCell>
                <TableCell>
                  <div class="flex gap-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      @click="
                        router.push({
                          path: `/packages/${encodeURIComponent(
                            pkg.package_id.registry,
                          )}/${encodeURIComponent(pkg.package_id.name)}`,
                        })
                      "
                    >
                      {{ t("common.details") }}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      @click="
                        router.push({
                          path: `/packages/${encodeURIComponent(
                            pkg.package_id.registry,
                          )}/${encodeURIComponent(pkg.package_id.name)}`,
                          query: {
                            version: pkg.package_id.version,
                            ...(pkg.package_id.artifact
                              ? { artifact: pkg.package_id.artifact }
                              : {}),
                          },
                        })
                      "
                    >
                      {{ t("common.artifact") }}
                    </Button>
                  </div>
                </TableCell>
                <TableCell class="text-right">
                  <div class="flex gap-1 justify-end">
                    <Button
                      v-if="pkg.status.status === 'blocked'"
                      variant="outline"
                      size="sm"
                      @click="unblock(pkg)"
                    >
                      {{ t("common.unblock") }}
                    </Button>
                    <Button
                      v-else
                      variant="outline"
                      size="sm"
                      @click="
                        blockReason = '';
                        pending = { kind: 'block-one', pkg };
                      "
                    >
                      {{ t("common.block") }}
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      class="text-destructive hover:text-destructive hover:bg-accent"
                      @click="pending = { kind: 'delete-one', pkg }"
                    >
                      {{ t("common.delete") }}
                    </Button>
                  </div>
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
        </AsyncState>
      </CardContent>
    </Card>
  </div>

  <DestructiveConfirm
    v-if="confirmProps"
    :open="pending !== null"
    v-bind="confirmProps"
    :loading="bulkLoading"
    :error="actionError"
    @update:open="(open: boolean) => !open && (pending = null)"
    @confirm="runPending"
  >
    <template v-if="pending?.kind === 'block-one' || pending?.kind === 'bulk-block'">
      <div class="space-y-1">
        <Label for="block-reason">{{ t("common.reason") }}</Label>
        <Input
          id="block-reason"
          v-model="blockReason"
          placeholder="CVE-2025-XXXX or policy violation"
        />
      </div>
    </template>
  </DestructiveConfirm>
</template>
