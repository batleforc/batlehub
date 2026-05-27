<script setup lang="ts">
import { computed, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { blockPackage, unblockPackage, bulkBlockPackages, bulkUnblockPackages, listRegistries } from "@/client/sdk.gen";
import type { RegistryInfo } from "@/client/types.gen";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";

import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell,
} from "@/components/ui/table";

interface VersionStatus {
  status: "available";
}
interface BlockedStatus {
  status: "blocked";
  reason: string;
  blocked_by: string;
  blocked_at: string;
}
interface PackageVersionDetail {
  id: string;
  version: string;
  artifact: string | null;
  status: VersionStatus | BlockedStatus;
  storage_key: string;
  storage_backend: string | null;
  cached: boolean;
  cached_at: string | null;
  access_count: number;
  last_accessed: string | null;
  last_accessed_by: string | null;
}
interface PackageEventDto {
  id: string;
  user_id: string | null;
  user_role: string;
  version: string;
  artifact: string | null;
  action: string;
  outcome: string;
  deny_reason: string | null;
  timestamp: string;
}
interface PackageDetailResponse {
  registry: string;
  name: string;
  versions: PackageVersionDetail[];
  recent_events: PackageEventDto[];
}

function isPreRelease(version: string): boolean {
  return version.includes("-");
}

const route = useRoute();
const router = useRouter();

function viewArtifact(v: PackageVersionDetail) {
  router.push({
    path: "/packages/detail",
    query: {
      registry: registry.value,
      name: name.value,
      version: v.version,
      ...(v.artifact ? { artifact: v.artifact } : {}),
    },
  });
}
const { token } = useAuth();

const registry = computed(() => String(route.query.registry ?? ""));
const name = computed(() => String(route.query.name ?? ""));

const API_BASE = import.meta.env.VITE_API_BASE_URL ?? "";

const { data: registriesList } = useApi<RegistryInfo[]>(
  () => listRegistries() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

const registryType = computed(() => {
  return registriesList.value?.find(r => r.name === registry.value)?.type ?? null;
});

const { data, error, loading, reload } = useApi<PackageDetailResponse>(
  () =>
    fetch(
      `${API_BASE}/api/v1/admin/packages/detail?registry=${encodeURIComponent(registry.value)}&name=${encodeURIComponent(name.value)}`,
      { headers: token.value ? { Authorization: `Bearer ${token.value}` } : {} },
    ).then(async (r) => {
      if (!r.ok) throw new Error(await r.text());
      return { data: await r.json() };
    }) as Promise<{ data?: unknown; error?: unknown }>,
  [token, registry, name],
);

interface BetaChannelMemberDto {
  principal_type: string;
  principal_id: string;
  granted_by: string | null;
}

const {
  data: betaMembers,
  loading: betaLoading,
  reload: reloadBeta,
} = useApi<BetaChannelMemberDto[]>(
  () => {
    if (!registry.value) return Promise.resolve({ data: [] }) as Promise<{ data?: unknown; error?: unknown }>;
    return fetch(
      `${API_BASE}/api/v1/admin/registries/${encodeURIComponent(registry.value)}/beta-channel`,
      { headers: token.value ? { Authorization: `Bearer ${token.value}` } : {} },
    ).then(async (r) => {
      if (!r.ok) throw new Error(await r.text());
      return { data: await r.json() };
    }) as Promise<{ data?: unknown; error?: unknown }>;
  },
  [token, registry],
);

const betaExpanded = ref(false);

const upstreamUrl = computed(() => {
  if (!registry.value || !name.value) return null;
  switch (registryType.value) {
    case "github": return `https://github.com/${name.value}`;
    case "npm":    return `https://www.npmjs.com/package/${name.value}`;
    case "cargo":  return `https://crates.io/crates/${name.value}`;
    default:       return null;
  }
});

function fmtDate(iso: string | null) {
  if (!iso) return "—";
  return new Date(iso).toLocaleString();
}

function fmtAction(action: string) {
  return { download: "Download", view_metadata: "View metadata", block: "Block", unblock: "Unblock" }[action] ?? action;
}

async function doBlock(v: PackageVersionDetail) {
  const reason = window.prompt("Block reason:");
  if (!reason) return;
  await blockPackage({
    body: {
      registry: registry.value,
      name: name.value,
      version: v.version,
      artifact: v.artifact ?? undefined,
      reason,
    },
  });
  reload();
}

async function doUnblock(v: PackageVersionDetail) {
  await unblockPackage({
    body: {
      registry: registry.value,
      name: name.value,
      version: v.version,
      artifact: v.artifact ?? undefined,
    },
  });
  reload();
}

async function doInvalidate(v: PackageVersionDetail) {
  if (!confirm(`Purge cached artifact for v${v.version}? The next download will re-fetch from upstream.`)) return;
  await fetch(
    `${API_BASE}/api/v1/admin/packages/invalidate`,
    {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        ...(token.value ? { Authorization: `Bearer ${token.value}` } : {}),
      },
      body: JSON.stringify({
        registry: registry.value,
        name: name.value,
        version: v.version,
        artifact: v.artifact ?? undefined,
      }),
    },
  );
  reload();
}

// ── Multi-select ──────────────────────────────────────────────────────────────

const selectedVersionIds = ref<Set<string>>(new Set());
const bulkLoading = ref(false);
const bulkResultMsg = ref<string | null>(null);

const allVersionsSelected = computed(
  () =>
    (data.value?.versions.length ?? 0) > 0 &&
    (data.value?.versions ?? []).every((v) => selectedVersionIds.value.has(v.id)),
);

function toggleAllVersions() {
  if (allVersionsSelected.value) {
    selectedVersionIds.value = new Set();
  } else {
    selectedVersionIds.value = new Set((data.value?.versions ?? []).map((v) => v.id));
  }
}

function toggleVersion(v: PackageVersionDetail) {
  if (selectedVersionIds.value.has(v.id)) selectedVersionIds.value.delete(v.id);
  else selectedVersionIds.value.add(v.id);
  selectedVersionIds.value = new Set(selectedVersionIds.value);
}

const selectedVersions = computed(() =>
  (data.value?.versions ?? []).filter((v) => selectedVersionIds.value.has(v.id)),
);

async function bulkBlockVersions() {
  const reason = window.prompt(`Block reason for ${selectedVersionIds.value.size} version(s):`);
  if (!reason) return;
  bulkLoading.value = true;
  bulkResultMsg.value = null;
  try {
    const res = await bulkBlockPackages({
      body: {
        items: selectedVersions.value.map((v) => ({
          registry: registry.value,
          name: name.value,
          version: v.version,
          artifact: v.artifact ?? null,
          reason,
        })),
      },
    });
    const r = res.data;
    if (r) bulkResultMsg.value = `Blocked ${r.succeeded_count} version(s)${r.failed_count ? `, ${r.failed_count} failed` : ""}.`;
  } finally {
    bulkLoading.value = false;
    selectedVersionIds.value = new Set();
    reload();
  }
}

async function bulkUnblockVersions() {
  if (!confirm(`Unblock ${selectedVersionIds.value.size} selected version(s)?`)) return;
  bulkLoading.value = true;
  bulkResultMsg.value = null;
  try {
    const res = await bulkUnblockPackages({
      body: {
        items: selectedVersions.value.map((v) => ({
          registry: registry.value,
          name: name.value,
          version: v.version,
          artifact: v.artifact ?? null,
        })),
      },
    });
    const r = res.data;
    if (r) bulkResultMsg.value = `Unblocked ${r.succeeded_count} version(s)${r.failed_count ? `, ${r.failed_count} failed` : ""}.`;
  } finally {
    bulkLoading.value = false;
    selectedVersionIds.value = new Set();
    reload();
  }
}
</script>

<template>
  <div class="space-y-4">
    <!-- Back -->
    <div class="flex items-center gap-3">
      <Button variant="ghost" size="sm" @click="router.back()">← Back</Button>
      <span class="text-muted-foreground text-sm">/</span>
      <span class="font-mono text-sm">{{ registry }}/{{ name }}</span>
    </div>

    <p v-if="loading" class="text-sm text-muted-foreground">Loading…</p>
    <p v-else-if="error" class="text-sm text-destructive">{{ error }}</p>

    <template v-else-if="data">
      <!-- Header card -->
      <Card>
        <CardHeader>
          <CardTitle class="text-xl font-mono">{{ data.name }}</CardTitle>
        </CardHeader>
        <CardContent class="space-y-1 text-sm">
          <div><span class="text-muted-foreground w-28 inline-block">Registry</span><Badge variant="outline">{{ data.registry }}</Badge></div>
          <div>
            <span class="text-muted-foreground w-28 inline-block">Upstream</span>
            <a v-if="upstreamUrl" :href="upstreamUrl" target="_blank" rel="noopener noreferrer"
               class="text-primary underline-offset-2 hover:underline font-mono text-xs">
              {{ upstreamUrl }}
            </a>
            <span v-else class="text-muted-foreground">—</span>
          </div>
          <div><span class="text-muted-foreground w-28 inline-block">Versions</span>{{ data.versions.length }}</div>
        </CardContent>
      </Card>

      <!-- Bulk action bar for versions -->
      <div
        v-if="selectedVersionIds.size > 0"
        class="sticky top-16 z-30 flex items-center gap-3 rounded-lg border bg-card px-4 py-2.5 shadow-sm"
      >
        <span class="text-sm font-medium">{{ selectedVersionIds.size }} version(s) selected</span>
        <Button size="sm" variant="destructive" :disabled="bulkLoading" @click="bulkBlockVersions">
          Block selected
        </Button>
        <Button size="sm" variant="outline" :disabled="bulkLoading" @click="bulkUnblockVersions">
          Unblock selected
        </Button>
        <Button size="sm" variant="ghost" @click="selectedVersionIds = new Set()">Clear</Button>
        <span v-if="bulkResultMsg" class="text-xs text-muted-foreground ml-auto">{{ bulkResultMsg }}</span>
      </div>

      <!-- Versions table -->
      <Card>
        <CardHeader>
          <CardTitle class="text-base">Versions &amp; artifacts</CardTitle>
        </CardHeader>
        <CardContent class="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead class="w-8">
                  <input type="checkbox" :checked="allVersionsSelected" @change="toggleAllVersions" class="cursor-pointer" />
                </TableHead>
                <TableHead>Version</TableHead>
                <TableHead>Artifact</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Cached</TableHead>
                <TableHead>Downloads</TableHead>
                <TableHead>Storage</TableHead>
                <TableHead>Last accessed</TableHead>
                <TableHead>Last pulled by</TableHead>
                <TableHead class="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow
                v-for="v in data.versions"
                :key="v.id"
                :class="v.status.status === 'blocked' ? 'bg-destructive/5' : ''"
              >
                <TableCell class="w-8">
                  <input
                    type="checkbox"
                    :checked="selectedVersionIds.has(v.id)"
                    @change="toggleVersion(v)"
                    class="cursor-pointer"
                  />
                </TableCell>
                <TableCell class="font-mono text-xs">
                  {{ v.version }}
                  <Badge v-if="isPreRelease(v.version)" variant="outline" class="ml-1 text-xs align-middle">pre-release</Badge>
                </TableCell>
                <TableCell class="font-mono text-xs text-muted-foreground">{{ v.artifact ?? "—" }}</TableCell>
                <TableCell>
                  <div class="space-y-0.5">
                    <Badge :variant="v.status.status === 'blocked' ? 'destructive' : 'secondary'">
                      {{ v.status.status === "blocked" ? "Blocked" : "Available" }}
                    </Badge>
                    <p v-if="v.status.status === 'blocked'" class="text-xs text-muted-foreground max-w-[180px] truncate" :title="(v.status as BlockedStatus).reason">
                      {{ (v.status as BlockedStatus).reason }}
                    </p>
                  </div>
                </TableCell>
                <TableCell>
                  <Badge :variant="v.cached ? 'default' : 'outline'" class="text-xs">
                    {{ v.cached ? "Cached" : "Not cached" }}
                  </Badge>
                  <p v-if="v.cached_at" class="text-xs text-muted-foreground mt-0.5">{{ fmtDate(v.cached_at) }}</p>
                  <p class="text-xs text-muted-foreground font-mono mt-0.5">{{ v.storage_key }}</p>
                </TableCell>
                <TableCell class="text-right tabular-nums">{{ v.access_count }}</TableCell>
                <TableCell>
                  <Badge v-if="v.storage_backend" variant="outline" class="text-xs font-mono">{{ v.storage_backend }}</Badge>
                  <span v-else class="text-muted-foreground text-sm">—</span>
                </TableCell>
                <TableCell class="text-xs">{{ fmtDate(v.last_accessed) }}</TableCell>
                <TableCell class="text-sm">
                  <span v-if="v.last_accessed_by" class="font-medium">{{ v.last_accessed_by }}</span>
                  <span v-else-if="v.access_count > 0" class="text-muted-foreground italic">anonymous</span>
                  <span v-else class="text-muted-foreground">—</span>
                </TableCell>
                <TableCell class="text-right">
                  <div class="flex justify-end gap-2">
                    <Button variant="ghost" size="sm" @click="viewArtifact(v)">View</Button>
                    <Button
                      v-if="v.cached"
                      variant="outline"
                      size="sm"
                      @click="doInvalidate(v)"
                    >
                      Purge cache
                    </Button>
                    <Button
                      v-if="v.status.status === 'blocked'"
                      variant="outline"
                      size="sm"
                      @click="doUnblock(v)"
                    >
                      Unblock
                    </Button>
                    <Button
                      v-else
                      variant="destructive"
                      size="sm"
                      @click="doBlock(v)"
                    >
                      Block
                    </Button>
                  </div>
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
          <p v-if="data.versions.length === 0" class="p-6 text-sm text-muted-foreground text-center">No versions tracked yet.</p>
        </CardContent>
      </Card>

      <!-- Beta channel access -->
      <Card>
        <CardHeader>
          <div class="flex items-center justify-between">
            <button
              class="flex items-center gap-2 text-base font-semibold hover:text-primary transition-colors"
              @click="betaExpanded = !betaExpanded"
            >
              Beta Channel Access
              <span class="text-muted-foreground text-xs font-normal">
                {{ betaExpanded ? "▲ hide" : "▼ show" }}
              </span>
              <Badge v-if="betaMembers && betaMembers.length > 0" variant="secondary" class="text-xs ml-1">
                {{ betaMembers.length }} member{{ betaMembers.length > 1 ? "s" : "" }}
              </Badge>
            </button>
            <Button
              v-if="betaExpanded"
              variant="outline"
              size="sm"
              :disabled="betaLoading"
              @click="reloadBeta"
            >
              {{ betaLoading ? "Loading…" : "Refresh" }}
            </Button>
          </div>
        </CardHeader>
        <CardContent v-if="betaExpanded" class="p-0">
          <p class="px-6 py-2 text-xs text-muted-foreground border-b">
            Pre-release versions (marked <span class="font-mono">pre-release</span> above) are only accessible to the users and groups listed here.
          </p>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Type</TableHead>
                <TableHead>Principal ID</TableHead>
                <TableHead>Granted by</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow
                v-for="m in betaMembers"
                :key="m.principal_type + ':' + m.principal_id"
              >
                <TableCell>
                  <Badge :variant="m.principal_type === 'user' ? 'default' : 'secondary'" class="text-xs capitalize">
                    {{ m.principal_type }}
                  </Badge>
                </TableCell>
                <TableCell class="font-mono text-sm">{{ m.principal_id }}</TableCell>
                <TableCell class="text-sm text-muted-foreground">{{ m.granted_by ?? "—" }}</TableCell>
              </TableRow>
            </TableBody>
          </Table>
          <p
            v-if="!betaMembers || betaMembers.length === 0"
            class="p-6 text-sm text-muted-foreground text-center"
          >
            No beta channel members — pre-release versions are not accessible to anyone.
          </p>
        </CardContent>
      </Card>

      <!-- Recent events -->
      <Card>
        <CardHeader>
          <CardTitle class="text-base">Recent access events</CardTitle>
        </CardHeader>
        <CardContent class="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>When</TableHead>
                <TableHead>User</TableHead>
                <TableHead>Role</TableHead>
                <TableHead>Version</TableHead>
                <TableHead>Artifact</TableHead>
                <TableHead>Action</TableHead>
                <TableHead>Outcome</TableHead>
                <TableHead>Reason</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow v-for="ev in data.recent_events" :key="ev.id">
                <TableCell class="text-xs tabular-nums whitespace-nowrap">{{ fmtDate(ev.timestamp) }}</TableCell>
                <TableCell class="text-sm">
                  <span v-if="ev.user_id">{{ ev.user_id }}</span>
                  <span v-else class="text-muted-foreground italic">anonymous</span>
                </TableCell>
                <TableCell><Badge variant="outline" class="text-xs capitalize">{{ ev.user_role }}</Badge></TableCell>
                <TableCell class="font-mono text-xs">{{ ev.version }}</TableCell>
                <TableCell class="font-mono text-xs text-muted-foreground">{{ ev.artifact ?? "—" }}</TableCell>
                <TableCell class="text-xs">{{ fmtAction(ev.action) }}</TableCell>
                <TableCell>
                  <Badge :variant="ev.outcome === 'denied' ? 'destructive' : 'secondary'" class="text-xs">
                    {{ ev.outcome }}
                  </Badge>
                </TableCell>
                <TableCell class="text-xs text-muted-foreground max-w-[200px] truncate" :title="ev.deny_reason ?? ''">
                  {{ ev.deny_reason ?? "—" }}
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
          <p v-if="data.recent_events.length === 0" class="p-6 text-sm text-muted-foreground text-center">No events recorded yet.</p>
        </CardContent>
      </Card>
    </template>
  </div>
</template>
