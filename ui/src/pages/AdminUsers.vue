<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useAuthFetch } from "@/composables/useAuthFetch";
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

interface BlockedUser {
  user_id: string;
  blocked_at: string;
  blocked_by: string;
  reason: string | null;
}

const { authFetch } = useAuthFetch();

const blockedUsers = ref<BlockedUser[]>([]);
const listLoading = ref(false);
const listError = ref<string | null>(null);

async function loadBlockedUsers() {
  listLoading.value = true;
  listError.value = null;
  try {
    const res = await authFetch("/api/v1/admin/users/blocked");
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    blockedUsers.value = (await res.json()) as BlockedUser[];
  } catch (e) {
    listError.value = e instanceof Error ? e.message : "Failed to load";
  } finally {
    listLoading.value = false;
  }
}

// ── Block user ──────────────────────────────────────────────────────────────

const blockDialogOpen = ref(false);
const blockForm = ref({ user_id: "", reason: "" });
const blockLoading = ref(false);
const blockError = ref<string | null>(null);

async function submitBlock() {
  const uid = blockForm.value.user_id.trim();
  if (!uid) return;
  blockLoading.value = true;
  blockError.value = null;
  try {
    const res = await authFetch(`/api/v1/admin/users/${encodeURIComponent(uid)}/block`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ reason: blockForm.value.reason.trim() || null }),
    });
    if (!res.ok) {
      const body = (await res.json().catch(() => ({}))) as { error?: string };
      throw new Error(body.error ?? `HTTP ${res.status}`);
    }
    blockDialogOpen.value = false;
    blockForm.value = { user_id: "", reason: "" };
    await loadBlockedUsers();
  } catch (e) {
    blockError.value = e instanceof Error ? e.message : "Unknown error";
  } finally {
    blockLoading.value = false;
  }
}

// ── Unblock user ────────────────────────────────────────────────────────────

const unblockTarget = ref<string | null>(null);
const unblockLoading = ref(false);
const unblockError = ref<string | null>(null);

async function confirmUnblock() {
  if (!unblockTarget.value) return;
  unblockLoading.value = true;
  unblockError.value = null;
  try {
    const res = await authFetch(
      `/api/v1/admin/users/${encodeURIComponent(unblockTarget.value)}/block`,
      { method: "DELETE" },
    );
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    unblockTarget.value = null;
    await loadBlockedUsers();
  } catch (e) {
    unblockError.value = e instanceof Error ? e.message : "Unknown error";
  } finally {
    unblockLoading.value = false;
  }
}

function fmtDate(iso: string): string {
  return new Date(iso).toLocaleString();
}

onMounted(() => {
  void loadBlockedUsers();
});
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-center justify-between">
      <div>
        <h1 class="text-2xl font-semibold">User Blocks</h1>
        <p class="text-sm text-muted-foreground mt-0.5">
          Block user accounts. Blocked users receive 401 responses on all authenticated requests.
        </p>
      </div>
      <div class="flex items-center gap-2">
        <Button variant="outline" size="sm" :disabled="listLoading" @click="loadBlockedUsers">
          {{ listLoading ? "Refreshing…" : "Refresh" }}
        </Button>
        <Button size="sm" @click="blockDialogOpen = true"> Block User </Button>
      </div>
    </div>

    <p v-if="listLoading && blockedUsers.length === 0" class="text-sm text-muted-foreground">
      Loading…
    </p>
    <p v-else-if="listError" class="text-sm text-destructive">{{ listError }}</p>

    <Card v-else>
      <CardHeader>
        <CardTitle class="text-base">Currently blocked users</CardTitle>
      </CardHeader>
      <CardContent class="p-0">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>User ID</TableHead>
              <TableHead>Reason</TableHead>
              <TableHead>Blocked at</TableHead>
              <TableHead>Blocked by</TableHead>
              <TableHead class="text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow v-for="entry in blockedUsers" :key="entry.user_id">
              <TableCell class="font-mono text-sm">{{ entry.user_id }}</TableCell>
              <TableCell
                class="text-sm text-muted-foreground max-w-[240px] truncate"
                :title="entry.reason ?? ''"
              >
                {{ entry.reason || "—" }}
              </TableCell>
              <TableCell class="text-xs">{{ fmtDate(entry.blocked_at) }}</TableCell>
              <TableCell class="text-xs font-mono">{{ entry.blocked_by }}</TableCell>
              <TableCell class="text-right">
                <Button variant="outline" size="sm" @click="unblockTarget = entry.user_id">
                  Unblock
                </Button>
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
        <p
          v-if="blockedUsers.length === 0"
          class="p-6 text-sm text-muted-foreground text-center"
        >
          No users are currently blocked.
        </p>
      </CardContent>
    </Card>
  </div>

  <!-- Block user dialog -->
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
        <h2 class="text-lg font-semibold">Block user</h2>
        <p class="text-sm text-muted-foreground mt-1">
          The user will receive 401 on all authenticated requests until unblocked.
        </p>
      </div>
      <div class="space-y-3">
        <div class="space-y-1.5">
          <Label for="userblock-id">User ID <span class="text-destructive">*</span></Label>
          <Input
            id="userblock-id"
            v-model="blockForm.user_id"
            placeholder="e.g. alice"
            class="font-mono"
          />
        </div>
        <div class="space-y-1.5">
          <Label for="userblock-reason">Reason</Label>
          <Input
            id="userblock-reason"
            v-model="blockForm.reason"
            placeholder="Optional reason"
          />
        </div>
      </div>
      <p v-if="blockError" class="text-sm text-destructive">{{ blockError }}</p>
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
          :disabled="blockLoading || !blockForm.user_id.trim()"
          @click="submitBlock"
        >
          {{ blockLoading ? "Blocking…" : "Block User" }}
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
          Unblock <span class="font-mono">{{ unblockTarget }}</span>?
        </h2>
        <p class="text-sm text-muted-foreground mt-1">
          This user will be immediately allowed to authenticate again.
        </p>
      </div>
      <p v-if="unblockError" class="text-sm text-destructive">{{ unblockError }}</p>
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
