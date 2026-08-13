<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { ref, computed } from "vue";
import { auditLog } from "@/client/sdk.gen";
import type { AuditLogResponse } from "@/client/types.gen";
import { useApi } from "@/composables/useApi";
import { API_BASE_URL } from "@/config";
import { useAuth } from "@/composables/useAuth";
import { formatDate } from "@/lib/format";
import SectionTabs from "@/components/admin/SectionTabs.vue";
import { PageHeader } from "@/components/ui/page-header";
import { OBSERVABILITY_TABS } from "@/config/adminSections";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
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

const { token } = useAuth();

/**
 * `GET /api/v1/admin/audit-log` answers with the paginated envelope
 * `{ items, total, page, per_page }` — not a bare array.
 *
 * This page declared `useApi<AccessEvent[]>` against a hand-written local
 * interface, so `data.value` held the envelope, `data.value.length` was
 * `undefined`, and the page rendered "No events recorded yet." over a full
 * page of events. On an *audit* surface that is the worst failure mode there
 * is: it does not look broken, it looks like nothing happened.
 *
 * The hand-written interface is gone with it. RFC 0004 R5 deleted four of
 * these mirrors from `registry-types.ts`; this one survived because it lived
 * in a page rather than in `lib/`, and the `as Promise<{ data?: unknown }>`
 * cast below is what let it disagree with the server in silence.
 */
const { data, error, loading, reload } = useApi<AuditLogResponse>(
  () => auditLog() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

/** The events themselves, out of the envelope. */
const events = computed(() => data.value?.items ?? []);

const exportFormat = ref<"json" | "csv">("csv");
const exporting = ref(false);

async function exportAuditLog() {
  exporting.value = true;
  try {
    const headers: Record<string, string> = {};
    if (token.value) headers["Authorization"] = `Bearer ${token.value}`;
    // `API_BASE_URL`, not a relative path: there is no `/api` proxy in front of
    // the SPA (CLAUDE.md), so a relative URL downloads the SPA's own index.html
    // on every deployment where the API is not same-origin.
    const params = new URLSearchParams({ format: exportFormat.value });
    // Export what is on screen. Handing someone a file that disagrees with the
    // table they were reading is worse than offering no export.
    if (userFilter.value.trim()) params.set("user_id", userFilter.value.trim());
    const url = `${API_BASE_URL}/api/v1/admin/audit-log/export?${params}`;
    const resp = await fetch(url, { headers });
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    const blob = await resp.blob();
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    const cd = resp.headers.get("Content-Disposition") ?? "";
    const match = cd.match(/filename="([^"]+)"/);
    a.download = match ? match[1] : `audit-log.${exportFormat.value}`;
    a.click();
    URL.revokeObjectURL(a.href);
  } finally {
    exporting.value = false;
  }
}

const userFilter = ref("");
const actionFilter = ref("");

const filteredItems = computed(() => {
  return events.value.filter((ev) => {
    const uq = userFilter.value.toLowerCase().trim();
    const aq = actionFilter.value.toLowerCase().trim();
    if (uq && !(ev.user_id ?? "").toLowerCase().includes(uq)) return false;
    if (aq && !ev.action.toLowerCase().includes(aq)) return false;
    return true;
  });
});

const actionOptions = computed(() =>
  [...new Set(events.value.map((e) => e.action))].sort((a, b) => a.localeCompare(b)),
);
</script>

<template>
  <div class="space-y-4">
    <SectionTabs :tabs="OBSERVABILITY_TABS" />
    <PageHeader variant="display">
      <template #title>
        {{ t("adminNav.auditLog") }}
        <!-- `total`, not the loaded page: the endpoint returns 100 rows by
             default and the count must not silently mean "the first 100". -->
        <span v-if="data?.total" class="font-mono text-base font-normal text-muted-foreground"
          >({{ data.total }})</span
        >
      </template>
    </PageHeader>
    <Card>
      <CardHeader class="space-y-3 pb-3">
        <div class="flex flex-row items-center justify-end space-y-0">
          <div class="flex gap-2 items-center">
            <select
              v-model="exportFormat"
              :aria-label="t('auditLog.exportFormat')"
              class="h-8 rounded-sm border border-input bg-transparent px-2 text-sm shadow-sm text-foreground"
            >
              <option value="csv">CSV</option>
              <option value="json">JSON</option>
            </select>
            <Button variant="outline" size="sm" :disabled="exporting" @click="exportAuditLog">
              {{ exporting ? t("adminSbom.exporting") : t("auditLog.export") }}
            </Button>
            <Button variant="outline" size="sm" @click="reload"> {{ t("common.refresh") }} </Button>
          </div>
        </div>
        <div class="flex gap-2 flex-wrap">
          <Input
            v-model="userFilter"
            :placeholder="t('auditLog.filterByUser')"
            :aria-label="t('auditLog.filterByUser2')"
            class="h-8 text-sm max-w-[200px]"
          />
          <select
            v-model="actionFilter"
            :aria-label="t('auditLog.filterByAction')"
            class="h-8 rounded-sm border border-input bg-transparent px-2 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring text-foreground"
          >
            <option value="">{{ t("auditLog.allActions") }}</option>
            <option v-for="a in actionOptions" :key="a" :value="a">
              {{ a }}
            </option>
          </select>
        </div>
      </CardHeader>
      <CardContent class="p-0">
        <p v-if="loading" class="p-6 text-sm text-muted-foreground">{{ t("auditLog.loading") }}</p>
        <p v-else-if="error" class="p-6 text-sm text-destructive">
          {{ error }}
        </p>

        <Table v-else-if="filteredItems.length">
          <TableHeader>
            <TableRow>
              <TableHead>{{ t("common.time") }}</TableHead>
              <TableHead>{{ t("common.user") }}</TableHead>
              <TableHead>{{ t("common.registry") }}</TableHead>
              <TableHead>{{ t("common.package") }}</TableHead>
              <TableHead>{{ t("common.action") }}</TableHead>
              <TableHead>{{ t("common.result") }}</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow
              v-for="ev in filteredItems"
              :key="ev.id"
              :class="ev.result.outcome === 'denied' ? 'bg-destructive/5' : ''"
            >
              <TableCell class="whitespace-nowrap text-xs tabular-nums">
                {{ formatDate(ev.timestamp) }}
              </TableCell>
              <TableCell class="text-sm font-mono">
                <span v-if="ev.user_id">{{ ev.user_id }}</span>
                <span v-else class="text-muted-foreground italic not-italic font-sans"
                  >anonymous</span
                >
              </TableCell>
              <!--
                `package_id` is null for account- and network-wide actions —
                blocking a user, blocking an IP, purging the trail itself.
                The hand-written interface this page used to carry declared it
                required, so the template dereferenced it unguarded; the
                generated type is honest about it, and those rows would have
                thrown on render once the envelope fix let them through.
              -->
              <TableCell class="font-mono text-xs">
                {{ ev.package_id?.registry ?? "—" }}
              </TableCell>
              <TableCell class="font-mono text-xs">
                <template v-if="ev.package_id">
                  {{ ev.package_id.name }}@{{ ev.package_id.version }}
                  <span v-if="ev.package_id.artifact" class="text-muted-foreground">
                    ({{ ev.package_id.artifact }})
                  </span>
                </template>
                <span v-else class="text-muted-foreground">{{ t("auditLog.accountWide") }}</span>
              </TableCell>
              <TableCell class="text-xs font-mono">
                {{ ev.action }}
              </TableCell>
              <TableCell class="max-w-[220px]">
                <Badge :variant="ev.result.outcome === 'denied' ? 'destructive' : 'secondary'">
                  {{
                    ev.result.outcome === "denied"
                      ? t("accessCheck.denied")
                      : t("accessCheck.allowed")
                  }}
                </Badge>
                <p
                  v-if="ev.result.outcome === 'denied'"
                  class="mt-0.5 text-xs text-muted-foreground truncate"
                  :title="ev.result.reason"
                >
                  {{ ev.result.reason }}
                </p>
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>

        <div v-else-if="!loading" class="p-6 text-sm text-muted-foreground text-center">
          {{
            userFilter || actionFilter
              ? t("auditLog.noEventsMatchTheCurrent")
              : t("packageEventsTable.noEventsRecordedYet")
          }}
        </div>
      </CardContent>
    </Card>
  </div>
</template>
