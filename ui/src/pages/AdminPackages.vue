<script setup lang="ts">
import { ref, computed, watch } from "vue";
import { useRouter } from "vue-router";
import { Package } from "@lucide/vue";
import SectionTabs from "@/components/admin/SectionTabs.vue";
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
import { useAuth } from "@/composables/useAuth";
import { useAuthFetch } from "@/composables/useAuthFetch";
import { formatDate as fmtDate } from "@/lib/format";
import { API_BASE_URL } from "@/config";
import { AsyncState } from "@/components/ui/async-state";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";

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

const {
  data: packagesResponse,
  error,
  loading,
  reload,
} = useApi<AdminPackageListResponse>(
  () =>
    listPackages({ query: { per_page: 1000 } }) as Promise<{
      data?: unknown;
      error?: unknown;
    }>,
  [token],
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
  const reason = globalThis.prompt("Block reason:");
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
  if (
    !confirm(
      `Delete package record and purge cached artifact for "${pkg.package_id.name}@${pkg.package_id.version}"? This cannot be undone.`,
    )
  )
    return;
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
  const reason = globalThis.prompt(`Block reason for ${selected.value.size} package(s):`);
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
  if (!confirm(`Unblock ${selected.value.size} selected package(s)?`)) return;
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
  if (
    !confirm(
      `Delete ${selected.value.size} selected package record(s) and purge their cached artifacts? This cannot be undone.`,
    )
  )
    return;
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

const showPreBlock = ref(false);
const preBlock = ref({
  registry: "",
  name: "",
  version: "",
  artifact: "",
  reason: "",
});
const preBlockError = ref<string | null>(null);
const preBlockLoading = ref(false);

watch(registries, (regs) => {
  if (regs && regs.length > 0 && !preBlock.value.registry) {
    preBlock.value.registry = regs[0].name;
  }
});

async function submitPreBlock() {
  if (!preBlock.value.name || !preBlock.value.version || !preBlock.value.reason) {
    preBlockError.value = "Name, version and reason are required.";
    return;
  }
  preBlockError.value = null;
  preBlockLoading.value = true;
  try {
    await blockPackage({
      body: {
        registry: preBlock.value.registry,
        name: preBlock.value.name,
        version: preBlock.value.version,
        artifact: preBlock.value.artifact || undefined,
        reason: preBlock.value.reason,
      },
    });
    const firstReg = registries.value?.[0]?.name ?? "";
    preBlock.value = {
      registry: firstReg,
      name: "",
      version: "",
      artifact: "",
      reason: "",
    };
    showPreBlock.value = false;
    reload();
  } catch (e: unknown) {
    preBlockError.value = extractMessage(e);
  } finally {
    preBlockLoading.value = false;
  }
}
</script>

<template>
  <div class="space-y-4">
    <SectionTabs :tabs="PACKAGES_TABS" />
    <!-- Pre-block form -->
    <Card>
      <CardHeader class="flex flex-row items-center justify-between space-y-0 pb-3">
        <CardTitle class="text-base"> Block a package </CardTitle>
        <Button variant="outline" size="sm" @click="showPreBlock = !showPreBlock">
          {{ showPreBlock ? "Cancel" : "Block new package" }}
        </Button>
      </CardHeader>

      <CardContent v-if="showPreBlock" class="space-y-4 pt-0">
        <p class="text-xs text-muted-foreground">
          Pre-emptively block a package before it is downloaded. The block takes effect immediately
          — any subsequent request for that package will be denied.
        </p>

        <div class="grid grid-cols-2 gap-3 sm:grid-cols-4">
          <div class="space-y-1">
            <Label for="pb-registry">Registry</Label>
            <select
              id="pb-registry"
              v-model="preBlock.registry"
              class="flex h-9 w-full rounded-sm border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring font-mono"
            >
              <option v-for="reg in registries" :key="reg.name" :value="reg.name">
                {{ reg.name }}
                <template v-if="reg.type !== reg.name"> ({{ reg.type }}) </template>
              </option>
            </select>
          </div>
          <div class="space-y-1 sm:col-span-2">
            <Label for="pb-name">Name</Label>
            <Input
              id="pb-name"
              v-model="preBlock.name"
              placeholder="owner/repo or lodash or serde"
              class="font-mono"
            />
          </div>
          <div class="space-y-1">
            <Label for="pb-version">Version / tag</Label>
            <Input
              id="pb-version"
              v-model="preBlock.version"
              placeholder="v1.2.3"
              class="font-mono"
            />
          </div>
        </div>

        <div class="grid grid-cols-2 gap-3">
          <div class="space-y-1">
            <Label for="pb-artifact"
              >Artifact <span class="text-muted-foreground">(optional)</span></Label
            >
            <Input
              id="pb-artifact"
              v-model="preBlock.artifact"
              placeholder="tarball / 123456789 / download"
              class="font-mono"
            />
          </div>
          <div class="space-y-1">
            <Label for="pb-reason">Reason</Label>
            <Input
              id="pb-reason"
              v-model="preBlock.reason"
              placeholder="CVE-2025-XXXX or policy violation"
            />
          </div>
        </div>

        <p v-if="preBlockError" class="text-xs text-destructive">
          {{ preBlockError }}
        </p>

        <Button :disabled="preBlockLoading" @click="submitPreBlock">
          {{ preBlockLoading ? "Blocking…" : "Block package" }}
        </Button>
      </CardContent>
    </Card>

    <!-- Bulk action bar -->
    <div
      v-if="selected.size > 0"
      class="sticky top-16 z-30 flex items-center gap-3 rounded-sm border bg-card px-4 py-2.5 shadow-sm"
    >
      <span class="text-sm font-medium">{{ selected.size }} selected</span>
      <Button size="sm" variant="destructive" :disabled="bulkLoading" @click="bulkBlock">
        Block selected
      </Button>
      <Button size="sm" variant="outline" :disabled="bulkLoading" @click="bulkUnblock">
        Unblock selected
      </Button>
      <Button size="sm" variant="destructive" :disabled="bulkLoading" @click="bulkDelete">
        Delete selected
      </Button>
      <Button size="sm" variant="ghost" @click="selected = new Set()"> Clear </Button>
      <span v-if="bulkResultMsg" class="text-xs text-muted-foreground ml-auto">{{
        bulkResultMsg
      }}</span>
    </div>

    <!-- Package list -->
    <Card>
      <CardHeader class="space-y-3 pb-3">
        <div class="flex flex-row items-center justify-between space-y-0">
          <CardTitle class="text-lg">
            All packages
            <span v-if="packages?.length" class="font-normal text-muted-foreground text-base ml-1"
              >({{ packages.length }})</span
            >
          </CardTitle>
          <Button variant="outline" size="sm" @click="reload"> Refresh </Button>
        </div>
        <Input
          v-model="search"
          placeholder="Filter by name, registry, or version…"
          aria-label="Filter packages"
          class="max-w-sm h-8 text-sm"
        />
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
                {{ search ? "No packages match your filter." : "No packages yet." }}
              </p>
              <p v-if="search" class="text-xs text-muted-foreground">Try clearing the filter.</p>
            </div>
          </template>

          <Table>
            <TableHeader>
              <TableRow>
                <TableHead class="w-8">
                  <input
                    type="checkbox"
                    aria-label="Select all packages"
                    :checked="allSelected"
                    class="cursor-pointer"
                    @change="toggleAll"
                  />
                </TableHead>
                <TableHead>Registry</TableHead>
                <TableHead>Name</TableHead>
                <TableHead>Version</TableHead>
                <TableHead>Artifact</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Last pulled</TableHead>
                <TableHead>Last pulled by</TableHead>
                <TableHead class="text-right"> Downloads </TableHead>
                <TableHead />
                <TableHead class="text-right"> Actions </TableHead>
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
                    :aria-label="`Select ${pkg.package_id.name}`"
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
                      {{ pkg.status.status === "blocked" ? "Blocked" : "Available" }}
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
                          path: '/admin/packages/detail',
                          query: {
                            registry: pkg.package_id.registry,
                            name: pkg.package_id.name,
                          },
                        })
                      "
                    >
                      Details
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      @click="
                        router.push({
                          path: '/packages/detail',
                          query: {
                            registry: pkg.package_id.registry,
                            name: pkg.package_id.name,
                            version: pkg.package_id.version,
                            ...(pkg.package_id.artifact
                              ? { artifact: pkg.package_id.artifact }
                              : {}),
                          },
                        })
                      "
                    >
                      Artifact
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
                      Unblock
                    </Button>
                    <Button v-else variant="destructive" size="sm" @click="block(pkg)">
                      Block
                    </Button>
                    <Button
                      variant="ghost"
                      size="sm"
                      class="text-destructive hover:text-destructive hover:bg-destructive/10"
                      @click="deletePkg(pkg)"
                    >
                      Delete
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
</template>
