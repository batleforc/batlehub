<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { ref } from "vue";
import { listBlockedIps, blockIp, unblockIp } from "@/client/sdk.gen";
import type { BlockedIpDto } from "@/lib/registry-types";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import SectionTabs from "@/components/admin/SectionTabs.vue";
import { SECURITY_TABS } from "@/config/adminSections";
import { PageHeader } from "@/components/ui/page-header";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import { Dialog } from "@/components/ui/dialog";

const { t } = useI18n();

const { token } = useAuth();

const { data, error, loading, reload } = useApi<BlockedIpDto[]>(
  () => listBlockedIps() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

const blockDialogOpen = ref(false);
const blockForm = ref({ ip: "", reason: "", duration_secs: 3600 });
const blockLoading = ref(false);
const blockError = ref<string | null>(null);

async function submitBlock() {
  if (!blockForm.value.ip.trim()) return;
  blockLoading.value = true;
  blockError.value = null;
  try {
    const { error: apiErr } = await blockIp({
      body: {
        ip: blockForm.value.ip.trim(),
        reason: blockForm.value.reason.trim() || undefined,
        duration_secs: blockForm.value.duration_secs || undefined,
      },
    });
    if (apiErr) throw new Error((apiErr as { message?: string })?.message ?? "API error");
    blockDialogOpen.value = false;
    blockForm.value = { ip: "", reason: "", duration_secs: 3600 };
    reload();
  } catch (e) {
    blockError.value = e instanceof Error ? e.message : "Unknown error";
  } finally {
    blockLoading.value = false;
  }
}

const unblockTarget = ref<string | null>(null);
const unblockLoading = ref(false);
const unblockError = ref<string | null>(null);

async function confirmUnblock() {
  if (!unblockTarget.value) return;
  unblockLoading.value = true;
  unblockError.value = null;
  try {
    const { error: apiErr } = await unblockIp({ path: { ip: unblockTarget.value } });
    if (apiErr) throw new Error((apiErr as { message?: string })?.message ?? "API error");
    unblockTarget.value = null;
    reload();
  } catch (e) {
    unblockError.value = e instanceof Error ? e.message : "Unknown error";
  } finally {
    unblockLoading.value = false;
  }
}

function fmtTs(secs: number): string {
  if (!secs) return "Permanent";
  return new Date(secs * 1000).toLocaleString();
}

function isExpired(unblock_at: number): boolean {
  return unblock_at > 0 && unblock_at * 1000 < Date.now();
}
</script>

<template>
  <div class="space-y-6">
    <SectionTabs :tabs="SECURITY_TABS" />
    <PageHeader
      :title="t('adminIpBlocks.ipBlocks')"
      :description="t('adminIpBlocks.manageManuallyBlockedIpAddresses')"
    >
      <template #actions>
        <Button variant="outline" size="sm" :disabled="loading" @click="reload">
          {{ loading ? t("adminHealth.refreshing") : t("common.refresh") }}
        </Button>
        <Button size="sm" @click="blockDialogOpen = true">{{ t("adminIpBlocks.blockIp") }}</Button>
      </template>
    </PageHeader>

    <p v-if="loading && !data" class="text-sm text-muted-foreground">
      {{ t("adminIpBlocks.loading") }}
    </p>
    <p v-else-if="error" class="text-sm text-destructive">
      {{ error }}
    </p>

    <Card v-else>
      <CardHeader>
        <CardTitle class="text-base">{{ t("adminIpBlocks.currentlyBlockedIps") }}</CardTitle>
      </CardHeader>
      <CardContent class="p-0">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>{{ t("adminIpBlocks.ipAddress") }}</TableHead>
              <TableHead>{{ t("common.reason") }}</TableHead>
              <TableHead>{{ t("adminIpBlocks.blockedAt") }}</TableHead>
              <TableHead>{{ t("adminIpBlocks.unblocksAt") }}</TableHead>
              <TableHead class="text-right"> {{ t("common.actions") }} </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow
              v-for="entry in data"
              :key="entry.ip"
              :class="isExpired(entry.unblock_at) ? 'opacity-50' : ''"
            >
              <TableCell class="font-mono text-sm">
                {{ entry.ip }}
              </TableCell>
              <TableCell
                class="text-sm text-muted-foreground max-w-[240px] truncate"
                :title="entry.reason"
              >
                {{ entry.reason || "—" }}
              </TableCell>
              <TableCell class="text-xs">
                {{ fmtTs(entry.blocked_at) }}
              </TableCell>
              <TableCell>
                <Badge v-if="isExpired(entry.unblock_at)" variant="outline" class="text-xs">
                  {{ t("common.expired") }}
                </Badge>
                <span v-else class="text-xs">{{ fmtTs(entry.unblock_at) }}</span>
              </TableCell>
              <TableCell class="text-right">
                <Button variant="outline" size="sm" @click="unblockTarget = entry.ip">
                  {{ t("common.unblock") }}
                </Button>
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
        <p v-if="!data || data.length === 0" class="p-6 text-sm text-muted-foreground text-center">
          {{ t("adminIpBlocks.noIpsAreCurrently") }}
        </p>
      </CardContent>
    </Card>
  </div>

  <!-- Block IP dialog -->
  <Dialog
    :open="blockDialogOpen"
    @update:open="
      (v) => {
        if (!v) {
          blockDialogOpen = false;
          blockError = null;
        }
      }
    "
  >
    <template #title>{{ t("adminIpBlocks.blockIpAddress") }}</template>
    <template #description>{{ t("adminIpBlocks.theIpWillBe") }}</template>
    <div class="space-y-4">
      <div class="space-y-3">
        <div class="space-y-1.5">
          <Label for="ipblock-ip">
            {{ t("adminIpBlocks.ipAddress") }} <span class="text-destructive">*</span>
          </Label>
          <Input
            id="ipblock-ip"
            v-model="blockForm.ip"
            placeholder="e.g. 203.0.113.42"
            class="font-mono"
          />
        </div>
        <div class="space-y-1.5">
          <Label for="ipblock-reason">{{ t("common.reason") }}</Label>
          <Input
            id="ipblock-reason"
            v-model="blockForm.reason"
            :placeholder="t('adminIpBlocks.optionalReason')"
          />
        </div>
        <div class="space-y-1.5">
          <Label for="ipblock-duration">{{ t("adminIpBlocks.durationSeconds") }}</Label>
          <Input
            id="ipblock-duration"
            v-model.number="blockForm.duration_secs"
            type="number"
            min="60"
            placeholder="3600"
          />
          <p class="text-xs text-muted-foreground">{{ t("adminIpBlocks.default3600S1") }}</p>
        </div>
      </div>
      <p v-if="blockError" class="text-sm text-destructive">
        {{ blockError }}
      </p>
      <div class="flex justify-end gap-2">
        <Button
          variant="outline"
          size="sm"
          :disabled="blockLoading"
          @click="
            blockDialogOpen = false;
            blockError = null;
          "
        >
          {{ t("common.cancel") }}
        </Button>
        <Button
          variant="destructive"
          size="sm"
          :disabled="blockLoading || !blockForm.ip.trim()"
          @click="submitBlock"
        >
          {{ blockLoading ? t("adminIpBlocks.blocking") : t("adminIpBlocks.blockIp") }}
        </Button>
      </div>
    </div>
  </Dialog>

  <!-- Unblock confirmation dialog -->
  <Dialog
    :open="unblockTarget !== null"
    @update:open="
      (v) => {
        if (!v) {
          unblockTarget = null;
          unblockError = null;
        }
      }
    "
  >
    <template #title
      >{{ t("common.unblock") }} <span class="font-mono">{{ unblockTarget }}</span
      >?</template
    >
    <template #description>{{ t("adminIpBlocks.thisIpWillBe") }}</template>
    <div class="space-y-4">
      <p v-if="unblockError" class="text-sm text-destructive">
        {{ unblockError }}
      </p>
      <div class="flex justify-end gap-2">
        <Button
          variant="outline"
          size="sm"
          :disabled="unblockLoading"
          @click="
            unblockTarget = null;
            unblockError = null;
          "
        >
          {{ t("common.cancel") }}
        </Button>
        <Button size="sm" :disabled="unblockLoading" @click="confirmUnblock">
          {{ unblockLoading ? t("adminIpBlocks.unblocking") : t("common.unblock") }}
        </Button>
      </div>
    </div>
  </Dialog>
</template>
