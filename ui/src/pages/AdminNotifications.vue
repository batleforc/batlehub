<script setup lang="ts">
import { ref, computed } from "vue";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
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

// ── Types ──────────────────────────────────────────────────────────────────────

type EventType = "package_published" | "package_yanked" | "package_unyanked" | "package_deleted";

interface Subscription {
  id: string;
  registry: string | null;
  package_name: string | null;
  event_types: EventType[];
  channel_name: string;
  created_by: string;
  created_at: string;
  enabled: boolean;
}

interface Channel {
  name: string;
}

interface InboundEvent {
  id: string;
  webhook_name: string;
  payload: unknown;
  source_ip: string | null;
  received_at: string;
  signature_valid: boolean | null;
}

type Tab = "subscriptions" | "inbound" | "channels";

// ── API calls ─────────────────────────────────────────────────────────────────

const BASE = "/api/v1/admin/notifications";

async function apiFetch(path: string, init?: RequestInit) {
  const res = await fetch(path, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${token.value}`,
      ...init?.headers,
    },
  });
  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new Error(`HTTP ${res.status}: ${body}`);
  }
  if (res.status === 204) return null;
  return res.json();
}

// ── State ─────────────────────────────────────────────────────────────────────

const activeTab = ref<Tab>("subscriptions");

const {
  data: subscriptions,
  error: subsError,
  loading: subsLoading,
  reload: reloadSubs,
} = useApi<Subscription[]>(() => apiFetch(`${BASE}/subscriptions`), [token]);

const {
  data: channelsResp,
  error: channelsError,
  loading: channelsLoading,
} = useApi<{ channels: Channel[] }>(() => apiFetch(`${BASE}/channels`), [token]);

const {
  data: inboundResp,
  error: inboundError,
  loading: inboundLoading,
  reload: reloadInbound,
} = useApi<{ events: InboundEvent[] }>(() => apiFetch(`${BASE}/inbound`), [token]);

const channels = computed(() => channelsResp.value?.channels ?? []);
const inboundEvents = computed(() => inboundResp.value?.events ?? []);

// ── Create / edit dialog ──────────────────────────────────────────────────────

const ALL_EVENT_TYPES: EventType[] = [
  "package_published",
  "package_yanked",
  "package_unyanked",
  "package_deleted",
];

const dialogOpen = ref(false);
const editingId = ref<string | null>(null);
const form = ref({
  registry: "",
  package_name: "",
  event_types: ["package_published"] as EventType[],
  channel_name: "",
  enabled: true,
});
const formLoading = ref(false);
const formError = ref<string | null>(null);

function openCreate() {
  editingId.value = null;
  form.value = {
    registry: "",
    package_name: "",
    event_types: ["package_published"],
    channel_name: "",
    enabled: true,
  };
  formError.value = null;
  dialogOpen.value = true;
}

function openEdit(sub: Subscription) {
  editingId.value = sub.id;
  form.value = {
    registry: sub.registry ?? "",
    package_name: sub.package_name ?? "",
    event_types: [...sub.event_types],
    channel_name: sub.channel_name,
    enabled: sub.enabled,
  };
  formError.value = null;
  dialogOpen.value = true;
}

function toggleEventType(et: EventType) {
  const idx = form.value.event_types.indexOf(et);
  if (idx === -1) form.value.event_types.push(et);
  else form.value.event_types.splice(idx, 1);
}

async function submitForm() {
  if (!form.value.channel_name.trim() || form.value.event_types.length === 0) return;
  formLoading.value = true;
  formError.value = null;
  try {
    const body = {
      registry: form.value.registry.trim() || null,
      package_name: form.value.package_name.trim() || null,
      event_types: form.value.event_types,
      channel_name: form.value.channel_name.trim(),
      enabled: form.value.enabled,
    };
    if (editingId.value) {
      await apiFetch(`${BASE}/subscriptions/${editingId.value}`, {
        method: "PUT",
        body: JSON.stringify(body),
      });
    } else {
      await apiFetch(`${BASE}/subscriptions`, { method: "POST", body: JSON.stringify(body) });
    }
    dialogOpen.value = false;
    reloadSubs();
  } catch (e) {
    formError.value = e instanceof Error ? e.message : "Unknown error";
  } finally {
    formLoading.value = false;
  }
}

// ── Delete ────────────────────────────────────────────────────────────────────

const deleteTarget = ref<string | null>(null);
const deleteLoading = ref(false);
const deleteError = ref<string | null>(null);

async function confirmDelete() {
  if (!deleteTarget.value) return;
  deleteLoading.value = true;
  deleteError.value = null;
  try {
    await apiFetch(`${BASE}/subscriptions/${deleteTarget.value}`, { method: "DELETE" });
    deleteTarget.value = null;
    reloadSubs();
  } catch (e) {
    deleteError.value = e instanceof Error ? e.message : "Unknown error";
  } finally {
    deleteLoading.value = false;
  }
}

// ── Toggle enabled ────────────────────────────────────────────────────────────

async function toggleEnabled(sub: Subscription) {
  try {
    await apiFetch(`${BASE}/subscriptions/${sub.id}`, {
      method: "PUT",
      body: JSON.stringify({
        ...sub,
        registry: sub.registry,
        package_name: sub.package_name,
        enabled: !sub.enabled,
      }),
    });
    reloadSubs();
  } catch {
    // ignore; reload will show correct state
    reloadSubs();
  }
}

// ── Test dispatch ─────────────────────────────────────────────────────────────

const testLoading = ref<string | null>(null);
const testMsg = ref<string | null>(null);

async function testSubscription(id: string) {
  testLoading.value = id;
  testMsg.value = null;
  try {
    await apiFetch(`${BASE}/subscriptions/${id}/test`, { method: "POST" });
    testMsg.value = "Test sent successfully.";
  } catch (e) {
    testMsg.value = `Test failed: ${e instanceof Error ? e.message : "Unknown error"}`;
  } finally {
    testLoading.value = null;
  }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtTs(ts: string) {
  return new Date(ts).toLocaleString();
}

function eventBadgeVariant(et: EventType): "default" | "secondary" | "destructive" | "outline" {
  if (et === "package_published") return "default";
  if (et === "package_yanked") return "destructive";
  if (et === "package_unyanked") return "secondary";
  return "outline";
}
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-center justify-between">
      <div>
        <h1 class="text-2xl font-semibold">Webhooks &amp; Notifications</h1>
        <p class="text-sm text-muted-foreground mt-0.5">
          Manage outbound notification subscriptions and monitor inbound webhook events.
        </p>
      </div>
    </div>

    <!-- Tab switcher -->
    <div class="flex gap-1 border-b">
      <button
        v-for="tab in ['subscriptions', 'channels', 'inbound'] as Tab[]"
        :key="tab"
        class="px-4 py-2 text-sm font-medium capitalize transition-colors"
        :class="
          activeTab === tab
            ? 'border-b-2 border-foreground text-foreground'
            : 'text-muted-foreground hover:text-foreground'
        "
        @click="activeTab = tab"
      >
        {{
          tab === "inbound" ? "Inbound Events" : tab === "channels" ? "Channels" : "Subscriptions"
        }}
      </button>
    </div>

    <!-- ── Subscriptions tab ── -->
    <div v-if="activeTab === 'subscriptions'" class="space-y-4">
      <div class="flex justify-end">
        <Button size="sm" @click="openCreate"> New Subscription </Button>
      </div>

      <p v-if="subsLoading && !subscriptions" class="text-sm text-muted-foreground">Loading…</p>
      <p v-else-if="subsError" class="text-sm text-destructive">{{ subsError }}</p>

      <Card v-else>
        <CardContent class="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Registry</TableHead>
                <TableHead>Package</TableHead>
                <TableHead>Events</TableHead>
                <TableHead>Channel</TableHead>
                <TableHead>Enabled</TableHead>
                <TableHead class="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow v-for="sub in subscriptions" :key="sub.id">
                <TableCell class="font-mono text-sm">{{ sub.registry ?? "*" }}</TableCell>
                <TableCell class="font-mono text-sm">{{ sub.package_name ?? "*" }}</TableCell>
                <TableCell>
                  <div class="flex flex-wrap gap-1">
                    <Badge
                      v-for="et in sub.event_types"
                      :key="et"
                      :variant="eventBadgeVariant(et)"
                      class="text-xs"
                    >
                      {{ et.replace("package_", "") }}
                    </Badge>
                  </div>
                </TableCell>
                <TableCell class="font-mono text-sm">{{ sub.channel_name }}</TableCell>
                <TableCell>
                  <Switch :model-value="sub.enabled" @update:model-value="toggleEnabled(sub)" />
                </TableCell>
                <TableCell class="text-right">
                  <div class="flex justify-end gap-2">
                    <Button
                      variant="outline"
                      size="sm"
                      :disabled="testLoading === sub.id"
                      @click="testSubscription(sub.id)"
                    >
                      {{ testLoading === sub.id ? "…" : "Test" }}
                    </Button>
                    <Button variant="outline" size="sm" @click="openEdit(sub)">Edit</Button>
                    <Button variant="destructive" size="sm" @click="deleteTarget = sub.id"
                      >Delete</Button
                    >
                  </div>
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
          <p
            v-if="!subscriptions || subscriptions.length === 0"
            class="p-6 text-sm text-muted-foreground text-center"
          >
            No subscriptions configured.
          </p>
        </CardContent>
      </Card>

      <p
        v-if="testMsg"
        class="text-sm"
        :class="testMsg.startsWith('Test failed') ? 'text-destructive' : 'text-green-600'"
      >
        {{ testMsg }}
      </p>
    </div>

    <!-- ── Channels tab ── -->
    <div v-if="activeTab === 'channels'" class="space-y-4">
      <p v-if="channelsLoading && !channelsResp" class="text-sm text-muted-foreground">Loading…</p>
      <p v-else-if="channelsError" class="text-sm text-destructive">{{ channelsError }}</p>
      <Card v-else>
        <CardHeader>
          <CardTitle class="text-base">Configured Channels</CardTitle>
        </CardHeader>
        <CardContent>
          <p class="text-xs text-muted-foreground mb-4">
            Channels are defined in <code class="font-mono text-xs">config.toml</code> under
            <code class="font-mono text-xs">[[notifications.channels]]</code>. URLs and secrets are
            not displayed here.
          </p>
          <div v-if="channels.length === 0" class="text-sm text-muted-foreground">
            No channels configured. Add
            <code class="font-mono text-xs">[[notifications.channels]]</code> entries to
            config.toml.
          </div>
          <div v-else class="flex flex-wrap gap-2">
            <Badge v-for="ch in channels" :key="ch.name" variant="outline">{{ ch.name }}</Badge>
          </div>
        </CardContent>
      </Card>
    </div>

    <!-- ── Inbound Events tab ── -->
    <div v-if="activeTab === 'inbound'" class="space-y-4">
      <div class="flex justify-end">
        <Button variant="outline" size="sm" :disabled="inboundLoading" @click="reloadInbound">
          {{ inboundLoading ? "Refreshing…" : "Refresh" }}
        </Button>
      </div>

      <p v-if="inboundLoading && !inboundResp" class="text-sm text-muted-foreground">Loading…</p>
      <p v-else-if="inboundError" class="text-sm text-destructive">{{ inboundError }}</p>

      <Card v-else>
        <CardContent class="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Webhook</TableHead>
                <TableHead>Received at</TableHead>
                <TableHead>Source IP</TableHead>
                <TableHead>Signature</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow v-for="ev in inboundEvents" :key="ev.id">
                <TableCell class="font-mono text-sm">{{ ev.webhook_name }}</TableCell>
                <TableCell class="text-xs">{{ fmtTs(ev.received_at) }}</TableCell>
                <TableCell class="font-mono text-xs">{{ ev.source_ip ?? "—" }}</TableCell>
                <TableCell>
                  <Badge v-if="ev.signature_valid === true" variant="default" class="text-xs"
                    >Valid</Badge
                  >
                  <Badge
                    v-else-if="ev.signature_valid === false"
                    variant="destructive"
                    class="text-xs"
                    >Invalid</Badge
                  >
                  <span v-else class="text-xs text-muted-foreground">—</span>
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
          <p
            v-if="inboundEvents.length === 0"
            class="p-6 text-sm text-muted-foreground text-center"
          >
            No inbound events received yet.
          </p>
        </CardContent>
      </Card>
    </div>
  </div>

  <!-- Create/Edit Subscription dialog -->
  <Dialog
    :open="dialogOpen"
    @update:open="
      (v) => {
        if (!v) dialogOpen = false;
      }
    "
  >
    <div class="space-y-4">
      <h2 class="text-lg font-semibold">
        {{ editingId ? "Edit Subscription" : "New Subscription" }}
      </h2>

      <div class="space-y-3">
        <div class="space-y-1.5">
          <Label
            >Registry
            <span class="text-muted-foreground text-xs">(leave blank for all)</span></Label
          >
          <Input v-model="form.registry" placeholder="e.g. my-cargo" class="font-mono" />
        </div>
        <div class="space-y-1.5">
          <Label
            >Package name
            <span class="text-muted-foreground text-xs">(leave blank for all)</span></Label
          >
          <Input v-model="form.package_name" placeholder="e.g. serde" class="font-mono" />
        </div>
        <div class="space-y-1.5">
          <Label>Event types <span class="text-destructive">*</span></Label>
          <div class="flex flex-wrap gap-2">
            <button
              v-for="et in ALL_EVENT_TYPES"
              :key="et"
              type="button"
              class="px-2 py-1 rounded border text-xs font-mono transition-colors"
              :class="
                form.event_types.includes(et)
                  ? 'bg-foreground text-background border-foreground'
                  : 'border-muted-foreground text-muted-foreground'
              "
              @click="toggleEventType(et)"
            >
              {{ et.replace("package_", "") }}
            </button>
          </div>
        </div>
        <div class="space-y-1.5">
          <Label>Channel <span class="text-destructive">*</span></Label>
          <Input
            v-model="form.channel_name"
            placeholder="e.g. my-slack"
            class="font-mono"
            list="channel-list"
          />
          <datalist id="channel-list">
            <option v-for="ch in channels" :key="ch.name" :value="ch.name" />
          </datalist>
        </div>
        <div v-if="editingId" class="flex items-center gap-2">
          <Switch v-model="form.enabled" />
          <Label>Enabled</Label>
        </div>
      </div>

      <p v-if="formError" class="text-sm text-destructive">{{ formError }}</p>
      <div class="flex justify-end gap-2">
        <Button variant="outline" size="sm" :disabled="formLoading" @click="dialogOpen = false"
          >Cancel</Button
        >
        <Button
          size="sm"
          :disabled="formLoading || !form.channel_name.trim() || form.event_types.length === 0"
          @click="submitForm"
        >
          {{ formLoading ? "Saving…" : editingId ? "Update" : "Create" }}
        </Button>
      </div>
    </div>
  </Dialog>

  <!-- Delete confirmation -->
  <Dialog
    :open="deleteTarget !== null"
    @update:open="
      (v) => {
        if (!v) {
          deleteTarget = null;
          deleteError = null;
        }
      }
    "
  >
    <div class="space-y-4">
      <h2 class="text-lg font-semibold">Delete subscription?</h2>
      <p class="text-sm text-muted-foreground">This action cannot be undone.</p>
      <p v-if="deleteError" class="text-sm text-destructive">{{ deleteError }}</p>
      <div class="flex justify-end gap-2">
        <Button variant="outline" size="sm" :disabled="deleteLoading" @click="deleteTarget = null"
          >Cancel</Button
        >
        <Button variant="destructive" size="sm" :disabled="deleteLoading" @click="confirmDelete">
          {{ deleteLoading ? "Deleting…" : "Delete" }}
        </Button>
      </div>
    </div>
  </Dialog>
</template>
