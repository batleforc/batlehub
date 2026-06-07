<script setup lang="ts">
import { ref } from "vue";
import { listBlockedIps, blockIp, unblockIp } from "@/client/sdk.gen";
import type { BlockedIpDto } from "@/lib/registry-types";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
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
import Dialog from "@/components/ui/dialog/Dialog.vue";

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
    <div class="flex items-center justify-between">
      <div>
        <h1 class="text-2xl font-semibold">IP Blocks</h1>
        <p class="text-sm text-muted-foreground mt-0.5">
          Manage manually blocked IP addresses. Blocked IPs receive 403 responses on all requests.
        </p>
      </div>
      <div class="flex items-center gap-2">
        <Button variant="outline" size="sm" :disabled="loading" @click="reload">
          {{ loading ? "Refreshing…" : "Refresh" }}
        </Button>
        <Button size="sm" @click="blockDialogOpen = true"> Block IP </Button>
      </div>
    </div>

    <p v-if="loading && !data" class="text-sm text-muted-foreground">Loading…</p>
    <p v-else-if="error" class="text-sm text-destructive">
      {{ error }}
    </p>

    <Card v-else>
      <CardHeader>
        <CardTitle class="text-base"> Currently blocked IPs </CardTitle>
      </CardHeader>
      <CardContent class="p-0">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>IP address</TableHead>
              <TableHead>Reason</TableHead>
              <TableHead>Blocked at</TableHead>
              <TableHead>Unblocks at</TableHead>
              <TableHead class="text-right"> Actions </TableHead>
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
                  Expired
                </Badge>
                <span v-else class="text-xs">{{ fmtTs(entry.unblock_at) }}</span>
              </TableCell>
              <TableCell class="text-right">
                <Button variant="outline" size="sm" @click="unblockTarget = entry.ip">
                  Unblock
                </Button>
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
        <p v-if="!data || data.length === 0" class="p-6 text-sm text-muted-foreground text-center">
          No IPs are currently blocked.
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
    <div class="space-y-4">
      <div>
        <h2 class="text-lg font-semibold">Block IP address</h2>
        <p class="text-sm text-muted-foreground mt-1">
          The IP will be blocked for the specified duration and receive 403 on all requests.
        </p>
      </div>
      <div class="space-y-3">
        <div class="space-y-1.5">
          <Label for="ipblock-ip">IP address <span class="text-destructive">*</span></Label>
          <Input
            id="ipblock-ip"
            v-model="blockForm.ip"
            placeholder="e.g. 203.0.113.42"
            class="font-mono"
          />
        </div>
        <div class="space-y-1.5">
          <Label for="ipblock-reason">Reason</Label>
          <Input id="ipblock-reason" v-model="blockForm.reason" placeholder="Optional reason" />
        </div>
        <div class="space-y-1.5">
          <Label for="ipblock-duration">Duration (seconds)</Label>
          <Input
            id="ipblock-duration"
            v-model.number="blockForm.duration_secs"
            type="number"
            min="60"
            placeholder="3600"
          />
          <p class="text-xs text-muted-foreground">Default: 3600 s (1 hour)</p>
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
          Cancel
        </Button>
        <Button
          variant="destructive"
          size="sm"
          :disabled="blockLoading || !blockForm.ip.trim()"
          @click="submitBlock"
        >
          {{ blockLoading ? "Blocking…" : "Block IP" }}
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
    <div class="space-y-4">
      <div>
        <h2 class="text-lg font-semibold">
          Unblock <span class="font-mono">{{ unblockTarget }}</span
          >?
        </h2>
        <p class="text-sm text-muted-foreground mt-1">
          This IP will be immediately allowed to send requests again.
        </p>
      </div>
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
          Cancel
        </Button>
        <Button size="sm" :disabled="unblockLoading" @click="confirmUnblock">
          {{ unblockLoading ? "Unblocking…" : "Unblock" }}
        </Button>
      </div>
    </div>
  </Dialog>
</template>
