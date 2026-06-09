<script setup lang="ts">
import { computed } from "vue";
import { useRoute, useRouter } from "vue-router";
import {
  blockPackage,
  unblockPackage,
  listPackages2,
  listRegistries,
  packageDetail,
} from "@/client/sdk.gen";
import type {
  RegistryInfo,
  PackageSummaryDto,
  PackageDetailResponse,
  PackageVersionDetail,
  PackageEventDto,
} from "@/client/types.gen";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";

type BlockedStatus = Extract<PackageVersionDetail["status"], { status: "blocked" }>;

const route = useRoute();
const router = useRouter();
const { token, isAdmin } = useAuth();

const registry = computed(() => String(route.query.registry ?? ""));
const name = computed(() => String(route.query.name ?? ""));
const version = computed(() => String(route.query.version ?? ""));
const artifact = computed(() => (route.query.artifact ? String(route.query.artifact) : null));

const { data: registriesList } = useApi<RegistryInfo[]>(
  () => listRegistries() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

const registryType = computed(
  () => registriesList.value?.find((r) => r.name === registry.value)?.type ?? null,
);

const upstreamUrl = computed(() => {
  if (!registry.value || !name.value) return null;
  switch (registryType.value) {
    case "github":
      return `https://github.com/${name.value}`;
    case "npm":
      return `https://www.npmjs.com/package/${name.value}`;
    case "cargo":
      return `https://crates.io/crates/${name.value}`;
    default:
      return null;
  }
});

const {
  data: adminData,
  error: adminError,
  loading: adminLoading,
  reload: adminReload,
} = useApi<PackageDetailResponse>(() => {
  if (!isAdmin.value) return Promise.resolve({ data: null as unknown });
  return packageDetail({
    query: { registry: registry.value, name: name.value },
  }) as Promise<{
    data?: unknown;
    error?: unknown;
  }>;
}, [token, registry, name, isAdmin]);

const {
  data: publicData,
  error: publicError,
  loading: publicLoading,
} = useApi<{ items: PackageSummaryDto[] }>(() => {
  if (isAdmin.value) return Promise.resolve({ data: null as unknown });
  return listPackages2({
    query: { registry: registry.value, name: name.value },
  }) as Promise<{
    data?: unknown;
    error?: unknown;
  }>;
}, [token, registry, name, isAdmin]);

const versionDetail = computed<PackageVersionDetail | null>(() => {
  if (!adminData.value) return null;
  return (
    adminData.value.versions.find(
      (v) => v.version === version.value && (v.artifact ?? null) === artifact.value,
    ) ?? null
  );
});

const publicSummary = computed<PackageSummaryDto | null>(() => {
  if (!publicData.value?.items) return null;
  return (
    publicData.value.items.find(
      (p) => p.version === version.value && (p.artifact ?? null) === artifact.value,
    ) ?? null
  );
});

const filteredEvents = computed<PackageEventDto[]>(() => {
  if (!adminData.value?.recent_events) return [];
  return adminData.value.recent_events.filter(
    (e) => e.version === version.value && (e.artifact ?? null) === artifact.value,
  );
});

const statusInfo = computed(
  () => versionDetail.value?.status ?? publicSummary.value?.status ?? null,
);

const accessCount = computed(
  () => versionDetail.value?.access_count ?? publicSummary.value?.access_count ?? 0,
);

const loading = computed(() => (isAdmin.value ? adminLoading.value : publicLoading.value));
const error = computed(() => (isAdmin.value ? adminError.value : publicError.value));
const notFound = computed(
  () =>
    !loading.value && !error.value && (isAdmin.value ? !versionDetail.value : !publicSummary.value),
);

function fmtDate(iso: string | null | undefined) {
  if (!iso) return "—";
  return new Date(iso).toLocaleString();
}

function fmtAction(action: string) {
  return (
    {
      download: "Download",
      view_metadata: "View metadata",
      block: "Block",
      unblock: "Unblock",
    }[action] ?? action
  );
}

async function doBlock() {
  const reason = globalThis.prompt("Block reason:");
  if (!reason) return;
  await blockPackage({
    body: {
      registry: registry.value,
      name: name.value,
      version: version.value,
      artifact: artifact.value ?? undefined,
      reason,
    },
  });
  adminReload();
}

async function doUnblock() {
  await unblockPackage({
    body: {
      registry: registry.value,
      name: name.value,
      version: version.value,
      artifact: artifact.value ?? undefined,
    },
  });
  adminReload();
}
</script>

<template>
  <div class="space-y-4">
    <!-- Breadcrumb -->
    <div class="flex items-center gap-3">
      <Button variant="ghost" size="sm" @click="router.back()"> ← Back </Button>
      <span class="text-muted-foreground text-sm">/</span>
      <span class="font-mono text-sm">
        {{ registry }}/{{ name }}/{{ version }}<template v-if="artifact">/{{ artifact }}</template>
      </span>
    </div>

    <p v-if="loading" class="text-sm text-muted-foreground">Loading…</p>
    <p v-else-if="error" class="text-sm text-destructive">
      {{ error }}
    </p>
    <p v-else-if="notFound" class="text-sm text-muted-foreground">Artifact not found.</p>

    <template v-else-if="statusInfo !== null">
      <!-- Header card -->
      <Card>
        <CardHeader>
          <CardTitle class="text-xl font-mono">
            {{ name }}
          </CardTitle>
        </CardHeader>
        <CardContent class="space-y-2 text-sm">
          <div class="flex flex-wrap gap-2 items-center">
            <Badge variant="outline">
              {{ registry }}
            </Badge>
            <Badge variant="secondary" class="font-mono">
              {{ version }}
            </Badge>
            <Badge v-if="artifact" variant="outline" class="font-mono text-xs">
              {{ artifact }}
            </Badge>
            <Badge :variant="statusInfo.status === 'blocked' ? 'destructive' : 'secondary'">
              {{ statusInfo.status === "blocked" ? "Blocked" : "Available" }}
            </Badge>
          </div>
          <p v-if="statusInfo.status === 'blocked'" class="text-xs text-destructive">
            {{ (statusInfo as BlockedStatus).reason }}
          </p>
          <div>
            <span class="text-muted-foreground w-28 inline-block">Upstream</span>
            <a
              v-if="upstreamUrl"
              :href="upstreamUrl"
              target="_blank"
              rel="noopener noreferrer"
              class="text-primary underline-offset-2 hover:underline font-mono text-xs"
            >
              {{ upstreamUrl }}
            </a>
            <span v-else class="text-muted-foreground">—</span>
          </div>
          <div>
            <span class="text-muted-foreground w-28 inline-block">Downloads</span>
            <span class="tabular-nums">{{ accessCount }}</span>
          </div>
        </CardContent>
      </Card>

      <!-- Admin: Cache & storage -->
      <Card v-if="isAdmin && versionDetail">
        <CardHeader>
          <CardTitle class="text-base"> Cache &amp; storage </CardTitle>
        </CardHeader>
        <CardContent class="space-y-2 text-sm">
          <div>
            <span class="text-muted-foreground w-32 inline-block">Status</span>
            <Badge :variant="versionDetail.cached ? 'default' : 'outline'">
              {{ versionDetail.cached ? "Cached" : "Not cached" }}
            </Badge>
          </div>
          <div v-if="versionDetail.cached_at">
            <span class="text-muted-foreground w-32 inline-block">Cached at</span>
            <span>{{ fmtDate(versionDetail.cached_at) }}</span>
          </div>
          <div>
            <span class="text-muted-foreground w-32 inline-block">Storage key</span>
            <code class="font-mono text-xs bg-muted px-1 py-0.5 rounded">{{
              versionDetail.storage_key
            }}</code>
          </div>
          <div>
            <span class="text-muted-foreground w-32 inline-block">Backend</span>
            <Badge v-if="versionDetail.storage_backend" variant="outline" class="font-mono text-xs">
              {{ versionDetail.storage_backend }}
            </Badge>
            <span v-else class="text-muted-foreground">—</span>
          </div>
          <div>
            <span class="text-muted-foreground w-32 inline-block">Last accessed</span>
            <span>{{ fmtDate(versionDetail.last_accessed) }}</span>
          </div>
          <div>
            <span class="text-muted-foreground w-32 inline-block">Last pulled by</span>
            <span v-if="versionDetail.last_accessed_by" class="font-medium">{{
              versionDetail.last_accessed_by
            }}</span>
            <span v-else-if="versionDetail.access_count > 0" class="text-muted-foreground italic"
              >anonymous</span
            >
            <span v-else class="text-muted-foreground">—</span>
          </div>
        </CardContent>
      </Card>

      <!-- Admin: Block / Unblock -->
      <div v-if="isAdmin && versionDetail" class="flex gap-2">
        <Button
          v-if="versionDetail.status.status === 'blocked'"
          variant="outline"
          @click="doUnblock"
        >
          Unblock
        </Button>
        <Button v-else variant="destructive" @click="doBlock"> Block </Button>
      </div>

      <!-- Admin: Recent events -->
      <Card v-if="isAdmin && versionDetail">
        <CardHeader>
          <CardTitle class="text-base"> Recent access events </CardTitle>
        </CardHeader>
        <CardContent class="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>When</TableHead>
                <TableHead>User</TableHead>
                <TableHead>Role</TableHead>
                <TableHead>Action</TableHead>
                <TableHead>Outcome</TableHead>
                <TableHead>Reason</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow v-for="ev in filteredEvents" :key="ev.id">
                <TableCell class="text-xs tabular-nums whitespace-nowrap">
                  {{ fmtDate(ev.timestamp) }}
                </TableCell>
                <TableCell class="text-sm">
                  <span v-if="ev.user_id">{{ ev.user_id }}</span>
                  <span v-else class="text-muted-foreground italic">anonymous</span>
                </TableCell>
                <TableCell>
                  <Badge variant="outline" class="text-xs capitalize">
                    {{ ev.user_role }}
                  </Badge>
                </TableCell>
                <TableCell class="text-xs">
                  {{ fmtAction(ev.action) }}
                </TableCell>
                <TableCell>
                  <Badge
                    :variant="ev.outcome === 'denied' ? 'destructive' : 'secondary'"
                    class="text-xs"
                  >
                    {{ ev.outcome }}
                  </Badge>
                </TableCell>
                <TableCell
                  class="text-xs text-muted-foreground max-w-[200px] truncate"
                  :title="ev.deny_reason ?? ''"
                >
                  {{ ev.deny_reason ?? "—" }}
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
          <p
            v-if="filteredEvents.length === 0"
            class="p-6 text-sm text-muted-foreground text-center"
          >
            No events recorded for this artifact.
          </p>
        </CardContent>
      </Card>
    </template>
  </div>
</template>
