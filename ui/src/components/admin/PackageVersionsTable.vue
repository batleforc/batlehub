<script setup lang="ts">
import { useI18n } from "vue-i18n";
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
import { formatDate as fmtDate } from "@/lib/format";
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

const { t } = useI18n();

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

function severityVariant(severity: string): "default" | "destructive" | "secondary" | "outline" {
  switch (severity) {
    case "critical":
    case "high":
      return "destructive";
    case "medium":
      return "default";
    default:
      return "secondary";
  }
}

function viewArtifact(v: PackageVersionDetail) {
  /* One canonical package URL; the version and artifact stay as query, since
     they select *within* a package rather than name a different one. */
  router.push({
    path: `/packages/${encodeURIComponent(props.registry)}/${encodeURIComponent(props.name)}`,
    query: {
      version: v.version,
      ...(v.artifact ? { artifact: v.artifact } : {}),
    },
  });
}

// ── Single-item actions ──────────────────────────────────────────────────────

async function doBlock(v: PackageVersionDetail) {
  const reason = globalThis.prompt(t("packageVersionsTable.blockReasonPrompt"));
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
  if (!confirm(t("packageVersionsTable.purgeArtifactConfirm", { version: v.version }))) return;
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

/** The `AdminPackages` sentence, over versions rather than packages. */
const BULK_KEYS = {
  blocked: "packageVersionsTable.bulkBlocked",
  unblocked: "packageVersionsTable.bulkUnblocked",
} as const;

function bulkOutcome(verb: keyof typeof BULK_KEYS, ok: number, failed: number): string {
  const done = t(BULK_KEYS[verb], { count: ok }, ok);
  return failed ? t("packageVersionsTable.bulkWithFailures", { done, failed }, failed) : done;
}

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
  const reason = globalThis.prompt(
    t(
      "packageVersionsTable.bulkBlockReasonPrompt",
      { count: selectedIds.value.size },
      selectedIds.value.size,
    ),
  );
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
    if (r) {
      bulkMsg.value = bulkOutcome("blocked", r.succeeded_count, r.failed_count);
    }
  } finally {
    bulkLoading.value = false;
    selectedIds.value = new Set();
    emit("reload");
  }
}

async function bulkUnblock() {
  if (
    !confirm(
      t(
        "packageVersionsTable.bulkUnblockConfirm",
        { count: selectedIds.value.size },
        selectedIds.value.size,
      ),
    )
  )
    return;
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
    if (r) {
      bulkMsg.value = bulkOutcome("unblocked", r.succeeded_count, r.failed_count);
    }
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
    <span class="text-sm font-medium">{{
      t("packageVersionsTable.versionsSelected", selectedIds.size)
    }}</span>
    <Button size="sm" variant="destructive" :disabled="bulkLoading" @click="bulkBlock">{{
      t("packageVersionsTable.blockSelected")
    }}</Button>
    <Button size="sm" variant="outline" :disabled="bulkLoading" @click="bulkUnblock">{{
      t("packageVersionsTable.unblockSelected")
    }}</Button>
    <Button size="sm" variant="ghost" @click="selectedIds = new Set()">{{
      t("common.clearAction")
    }}</Button>
    <span v-if="bulkMsg" class="text-xs text-muted-foreground ml-auto">{{ bulkMsg }}</span>
  </div>

  <!-- Versions table -->
  <Card>
    <CardHeader>
      <CardTitle class="text-base">{{ t("packageVersionsTable.versionsArtifacts") }}</CardTitle>
    </CardHeader>
    <CardContent class="p-0">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead class="w-8">
              <input
                type="checkbox"
                :aria-label="t('packageVersionsTable.selectAllVersions')"
                :checked="allSelected"
                class="cursor-pointer"
                @change="toggleAll"
              />
            </TableHead>
            <TableHead>{{ t("common.version") }}</TableHead>
            <TableHead>{{ t("common.artifact") }}</TableHead>
            <TableHead>{{ t("common.status") }}</TableHead>
            <TableHead>{{ t("common.security") }}</TableHead>
            <TableHead>{{ t("common.cached") }}</TableHead>
            <TableHead>{{ t("common.downloads") }}</TableHead>
            <TableHead>{{ t("common.storage") }}</TableHead>
            <TableHead>{{ t("packageVersionsTable.lastAccessed") }}</TableHead>
            <TableHead>{{ t("packageVersionsTable.lastPulledBy") }}</TableHead>
            <TableHead class="text-right">{{ t("common.actions") }}</TableHead>
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
                :aria-label="t('packageVersionsTable.selectVersion', { version: v.version })"
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
              <!-- In the version cell rather than a column of its own: RFC
                   0004-bis §6.1's verdict on this table is that it is already
                   too wide at 1440, so a licence that earned a twelfth column
                   would push the row verbs further off screen.

                   A dash when unknown, with the reason in the title. Rendering
                   nothing would let "we never read a manifest for this registry
                   type" look identical to "this package declares no licence",
                   which is the §2.4 defect — a blank that reads as a fact. -->
              <p
                class="text-muted-foreground truncate max-w-[180px]"
                :title="v.license ?? t('packageVersionsTable.licenseUnknownHelp')"
              >
                {{ v.license ?? t("packageVersionsTable.licenseUnknown") }}
              </p>
            </TableCell>
            <TableCell class="font-mono text-xs text-muted-foreground">{{
              v.artifact ?? "—"
            }}</TableCell>
            <TableCell>
              <div class="space-y-0.5">
                <Badge :variant="v.status.status === 'blocked' ? 'destructive' : 'secondary'">
                  {{
                    v.status.status === "blocked"
                      ? t("common.blocked")
                      : t("packageVersionsTable.available")
                  }}
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
            <TableCell class="text-sm">
              <div class="flex flex-wrap items-center gap-1">
                <span
                  v-for="vuln in v.vulnerabilities"
                  :key="vuln.osv_id"
                  :title="`${vuln.osv_id}: ${vuln.summary}${vuln.fixed_version ? t('packageVersionsTable.fixedIn', { version: vuln.fixed_version }) : ''}`"
                >
                  <Badge :variant="severityVariant(vuln.severity)" class="text-xs cursor-help">
                    {{ vuln.severity }}
                  </Badge>
                </span>
                <a
                  v-if="v.socket_badge_url"
                  :href="v.socket_badge_url"
                  target="_blank"
                  rel="noopener noreferrer"
                  :title="t('packageVersionsTable.supplyChainReportOn')"
                >
                  <img :src="v.socket_badge_url" alt="socket.dev" class="h-4" />
                </a>
                <span
                  v-if="v.vulnerabilities.length === 0 && !v.socket_badge_url"
                  class="text-muted-foreground"
                  >—</span
                >
              </div>
            </TableCell>
            <TableCell>
              <Badge :variant="v.cached ? 'default' : 'outline'" class="text-xs">
                {{ v.cached ? t("common.cached") : t("packageVersionsTable.notCached") }}
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
                <Button variant="ghost" size="sm" @click="viewArtifact(v)">{{
                  t("common.view")
                }}</Button>
                <Button v-if="v.cached" variant="outline" size="sm" @click="doInvalidate(v)">{{
                  t("packageVersionsTable.purgeCache")
                }}</Button>
                <Button
                  v-if="v.status.status === 'blocked'"
                  variant="outline"
                  size="sm"
                  @click="doUnblock(v)"
                  >{{ t("common.unblock") }}</Button
                >
                <Button v-else variant="destructive" size="sm" @click="doBlock(v)">{{
                  t("common.block")
                }}</Button>
              </div>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
      <p v-if="versions.length === 0" class="p-6 text-sm text-muted-foreground text-center">
        {{ t("packageVersionsTable.noVersionsTrackedYet") }}
      </p>
    </CardContent>
  </Card>
</template>
