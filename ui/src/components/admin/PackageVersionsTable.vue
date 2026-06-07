<script setup lang="ts">
import { ref, computed } from "vue";
import { useRouter } from "vue-router";
import {
  blockPackage,
  unblockPackage,
  bulkBlockPackages,
  bulkUnblockPackages,
  invalidatePackage,
} from "@/client/sdk.gen";
import type { PackageVersionDetail } from "@/client/types.gen";
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

const props = defineProps<{
  registry: string;
  name: string;
  versions: PackageVersionDetail[];
}>();

const emit = defineEmits<{ reload: [] }>();

const router = useRouter();

function isPreRelease(v: string) {
  return v.includes("-");
}

function fmtDate(iso: string | null | undefined) {
  if (!iso) return "—";
  return new Date(iso).toLocaleString();
}

function viewArtifact(v: PackageVersionDetail) {
  router.push({
    path: "/packages/detail",
    query: {
      registry: props.registry,
      name: props.name,
      version: v.version,
      ...(v.artifact ? { artifact: v.artifact } : {}),
    },
  });
}

// ── Single-item actions ──────────────────────────────────────────────────────

async function doBlock(v: PackageVersionDetail) {
  const reason = window.prompt("Block reason:");
  if (!reason) return;
  await blockPackage({
    body: {
      registry: props.registry,
      name: props.name,
      version: v.version,
      artifact: v.artifact ?? undefined,
      reason,
    },
  });
  emit("reload");
}

async function doUnblock(v: PackageVersionDetail) {
  await unblockPackage({
    body: {
      registry: props.registry,
      name: props.name,
      version: v.version,
      artifact: v.artifact ?? undefined,
    },
  });
  emit("reload");
}

async function doInvalidate(v: PackageVersionDetail) {
  if (
    !confirm(
      `Purge cached artifact for v${v.version}? The next download will re-fetch from upstream.`,
    )
  )
    return;
  await invalidatePackage({
    body: {
      registry: props.registry,
      name: props.name,
      version: v.version,
      artifact: v.artifact ?? undefined,
    },
  });
  emit("reload");
}

// ── Multi-select ──────────────────────────────────────────────────────────────

const selectedIds = ref<Set<string>>(new Set());
const bulkLoading = ref(false);
const bulkMsg = ref<string | null>(null);

const allSelected = computed(
  () => props.versions.length > 0 && props.versions.every((v) => selectedIds.value.has(v.id)),
);

function toggleAll() {
  selectedIds.value = allSelected.value ? new Set() : new Set(props.versions.map((v) => v.id));
}

function toggle(v: PackageVersionDetail) {
  if (selectedIds.value.has(v.id)) selectedIds.value.delete(v.id);
  else selectedIds.value.add(v.id);
  selectedIds.value = new Set(selectedIds.value);
}

const selected = computed(() => props.versions.filter((v) => selectedIds.value.has(v.id)));

async function bulkBlock() {
  const reason = window.prompt(`Block reason for ${selectedIds.value.size} version(s):`);
  if (!reason) return;
  bulkLoading.value = true;
  bulkMsg.value = null;
  try {
    const res = await bulkBlockPackages({
      body: {
        items: selected.value.map((v) => ({
          registry: props.registry,
          name: props.name,
          version: v.version,
          artifact: v.artifact ?? null,
          reason,
        })),
      },
    });
    const r = res.data;
    if (r)
      bulkMsg.value = `Blocked ${r.succeeded_count} version(s)${r.failed_count ? `, ${r.failed_count} failed` : ""}.`;
  } finally {
    bulkLoading.value = false;
    selectedIds.value = new Set();
    emit("reload");
  }
}

async function bulkUnblock() {
  if (!confirm(`Unblock ${selectedIds.value.size} selected version(s)?`)) return;
  bulkLoading.value = true;
  bulkMsg.value = null;
  try {
    const res = await bulkUnblockPackages({
      body: {
        items: selected.value.map((v) => ({
          registry: props.registry,
          name: props.name,
          version: v.version,
          artifact: v.artifact ?? null,
        })),
      },
    });
    const r = res.data;
    if (r)
      bulkMsg.value = `Unblocked ${r.succeeded_count} version(s)${r.failed_count ? `, ${r.failed_count} failed` : ""}.`;
  } finally {
    bulkLoading.value = false;
    selectedIds.value = new Set();
    emit("reload");
  }
}
</script>

<template>
  <!-- Bulk action bar -->
  <div
    v-if="selectedIds.size > 0"
    class="sticky top-16 z-30 flex items-center gap-3 rounded-sm border bg-card px-4 py-2.5 shadow-sm"
  >
    <span class="text-sm font-medium">{{ selectedIds.size }} version(s) selected</span>
    <Button size="sm" variant="destructive" :disabled="bulkLoading" @click="bulkBlock"
      >Block selected</Button
    >
    <Button size="sm" variant="outline" :disabled="bulkLoading" @click="bulkUnblock"
      >Unblock selected</Button
    >
    <Button size="sm" variant="ghost" @click="selectedIds = new Set()">Clear</Button>
    <span v-if="bulkMsg" class="text-xs text-muted-foreground ml-auto">{{ bulkMsg }}</span>
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
              <input
                type="checkbox"
                :checked="allSelected"
                class="cursor-pointer"
                @change="toggleAll"
              />
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
            v-for="v in versions"
            :key="v.id"
            :class="v.status.status === 'blocked' ? 'bg-destructive/5' : ''"
          >
            <TableCell class="w-8">
              <input
                type="checkbox"
                :checked="selectedIds.has(v.id)"
                class="cursor-pointer"
                @change="toggle(v)"
              />
            </TableCell>
            <TableCell class="font-mono text-xs">
              {{ v.version }}
              <Badge
                v-if="isPreRelease(v.version)"
                variant="outline"
                class="ml-1 text-xs align-middle"
                >pre-release</Badge
              >
            </TableCell>
            <TableCell class="font-mono text-xs text-muted-foreground">{{
              v.artifact ?? "—"
            }}</TableCell>
            <TableCell>
              <div class="space-y-0.5">
                <Badge :variant="v.status.status === 'blocked' ? 'destructive' : 'secondary'">
                  {{ v.status.status === "blocked" ? "Blocked" : "Available" }}
                </Badge>
                <p
                  v-if="v.status.status === 'blocked'"
                  class="text-xs text-muted-foreground max-w-[180px] truncate"
                  :title="(v.status as BlockedStatus).reason"
                >
                  {{ (v.status as BlockedStatus).reason }}
                </p>
              </div>
            </TableCell>
            <TableCell>
              <Badge :variant="v.cached ? 'default' : 'outline'" class="text-xs">
                {{ v.cached ? "Cached" : "Not cached" }}
              </Badge>
              <p v-if="v.cached_at" class="text-xs text-muted-foreground mt-0.5">
                {{ fmtDate(v.cached_at) }}
              </p>
              <p class="text-xs text-muted-foreground font-mono mt-0.5">{{ v.storage_key }}</p>
            </TableCell>
            <TableCell class="text-right tabular-nums">{{ v.access_count }}</TableCell>
            <TableCell>
              <Badge v-if="v.storage_backend" variant="outline" class="text-xs font-mono">{{
                v.storage_backend
              }}</Badge>
              <span v-else class="text-muted-foreground text-sm">—</span>
            </TableCell>
            <TableCell class="text-xs">{{ fmtDate(v.last_accessed) }}</TableCell>
            <TableCell class="text-sm">
              <span v-if="v.last_accessed_by" class="font-medium">{{ v.last_accessed_by }}</span>
              <span v-else-if="v.access_count > 0" class="text-muted-foreground italic"
                >anonymous</span
              >
              <span v-else class="text-muted-foreground">—</span>
            </TableCell>
            <TableCell class="text-right">
              <div class="flex justify-end gap-2">
                <Button variant="ghost" size="sm" @click="viewArtifact(v)">View</Button>
                <Button v-if="v.cached" variant="outline" size="sm" @click="doInvalidate(v)"
                  >Purge cache</Button
                >
                <Button
                  v-if="v.status.status === 'blocked'"
                  variant="outline"
                  size="sm"
                  @click="doUnblock(v)"
                  >Unblock</Button
                >
                <Button v-else variant="destructive" size="sm" @click="doBlock(v)">Block</Button>
              </div>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
      <p v-if="versions.length === 0" class="p-6 text-sm text-muted-foreground text-center">
        No versions tracked yet.
      </p>
    </CardContent>
  </Card>
</template>
