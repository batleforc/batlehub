<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { useAuth } from "@/composables/useAuth";
import { useBanner, type GlobalBanner } from "@/composables/useBanner";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

const { token } = useAuth();
const { banner } = useBanner();
const API_BASE = (import.meta as Record<string, unknown> & { env: Record<string, string> }).env.VITE_API_BASE_URL ?? "";

// ── State ─────────────────────────────────────────────────────────────────────

const hotReloadEnabled = ref<boolean | null>(null);
const pendingReload = ref<null | {
  id: string;
  created_at: string;
  expires_at: string;
  source: string;
  diff: {
    added_registries: string[];
    removed_registries: string[];
    changed_registries: { name: string; fields: string[] }[];
    access_config_changed: boolean;
    limits_changed: boolean;
  };
}>(null);
const changeHistory = ref<{
  id: string;
  triggered_by: string;
  triggered_at: string;
  status: string;
  summary: string;
  diff: unknown;
  error_msg: string | null;
}[]>([]);

const loadingPending = ref(false);
const loadingForce = ref(false);
const loadingApply = ref(false);
const loadingDiscard = ref(false);
const loadingHistory = ref(false);
const errorMsg = ref<string | null>(null);
const successMsg = ref<string | null>(null);
const expandedRow = ref<string | null>(null);

// Banner form
const bannerMessage = ref("");
const bannerLevel = ref<"info" | "warning" | "error">("info");
const loadingSetBanner = ref(false);
const loadingClearBanner = ref(false);

let pollTimer: ReturnType<typeof setInterval> | null = null;

// ── Helpers ───────────────────────────────────────────────────────────────────

function authHeaders(): HeadersInit {
  return token.value ? { Authorization: `Bearer ${token.value}` } : {};
}

async function apiFetch(path: string, opts: RequestInit = {}) {
  const resp = await fetch(`${API_BASE}${path}`, {
    ...opts,
    headers: { ...authHeaders(), ...(opts.headers ?? {}) },
  });
  if (!resp.ok) {
    const body = await resp.text().catch(() => "");
    throw new Error(body || `HTTP ${resp.status}`);
  }
  return resp;
}

async function fetchPending() {
  loadingPending.value = true;
  try {
    const resp = await fetch(`${API_BASE}/api/v1/admin/config/pending`, {
      headers: authHeaders(),
    });
    if (resp.status === 404) {
      pendingReload.value = null;
    } else if (resp.ok) {
      pendingReload.value = await resp.json();
    }
    hotReloadEnabled.value = resp.status !== 503;
  } catch {
    // ignore
  } finally {
    loadingPending.value = false;
  }
}

async function fetchHistory() {
  loadingHistory.value = true;
  try {
    const resp = await apiFetch("/api/v1/admin/config/changes?per_page=20");
    const data = await resp.json();
    changeHistory.value = data.items ?? [];
  } catch (e: unknown) {
    // non-fatal
    console.warn("config history fetch failed:", e);
  } finally {
    loadingHistory.value = false;
  }
}

async function forceReload() {
  loadingForce.value = true;
  errorMsg.value = null;
  successMsg.value = null;
  try {
    const resp = await apiFetch("/api/v1/admin/config/reload", { method: "POST" });
    const data = await resp.json();
    const diff = data.diff;
    successMsg.value = `Reloaded: +${diff.added_registries.length} -${diff.removed_registries.length} registries`;
    await fetchPending();
    await fetchHistory();
  } catch (e: unknown) {
    errorMsg.value = e instanceof Error ? e.message : String(e);
  } finally {
    loadingForce.value = false;
  }
}

async function applyPending() {
  loadingApply.value = true;
  errorMsg.value = null;
  successMsg.value = null;
  try {
    const resp = await apiFetch("/api/v1/admin/config/pending/apply", { method: "POST" });
    const data = await resp.json();
    const diff = data.diff;
    successMsg.value = `Applied: +${diff.added_registries.length} -${diff.removed_registries.length} registries`;
    pendingReload.value = null;
    await fetchHistory();
  } catch (e: unknown) {
    errorMsg.value = e instanceof Error ? e.message : String(e);
  } finally {
    loadingApply.value = false;
  }
}

async function discardPending() {
  loadingDiscard.value = true;
  errorMsg.value = null;
  try {
    await apiFetch("/api/v1/admin/config/pending", { method: "DELETE" });
    pendingReload.value = null;
  } catch (e: unknown) {
    errorMsg.value = e instanceof Error ? e.message : String(e);
  } finally {
    loadingDiscard.value = false;
  }
}

async function setBannerAction() {
  if (!bannerMessage.value.trim()) return;
  loadingSetBanner.value = true;
  errorMsg.value = null;
  try {
    await apiFetch("/api/v1/admin/banner", {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ message: bannerMessage.value, level: bannerLevel.value }),
    });
    successMsg.value = "Banner set";
    bannerMessage.value = "";
  } catch (e: unknown) {
    errorMsg.value = e instanceof Error ? e.message : String(e);
  } finally {
    loadingSetBanner.value = false;
  }
}

async function clearBannerAction() {
  loadingClearBanner.value = true;
  errorMsg.value = null;
  try {
    await apiFetch("/api/v1/admin/banner", { method: "DELETE" });
    successMsg.value = "Banner cleared";
  } catch (e: unknown) {
    errorMsg.value = e instanceof Error ? e.message : String(e);
  } finally {
    loadingClearBanner.value = false;
  }
}

const expiresIn = computed(() => {
  if (!pendingReload.value) return "";
  const secs = Math.max(
    0,
    Math.round((new Date(pendingReload.value.expires_at).getTime() - Date.now()) / 1000),
  );
  if (secs > 60) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
  return `${secs}s`;
});

onMounted(async () => {
  await Promise.all([fetchPending(), fetchHistory()]);
  pollTimer = setInterval(() => void fetchPending(), 5_000);
});
onUnmounted(() => { if (pollTimer) clearInterval(pollTimer); });
</script>

<template>
  <div class="space-y-6">
    <h1 class="text-2xl font-bold">Config Reload</h1>

    <!-- Status: hot reload disabled -->
    <Card v-if="hotReloadEnabled === false" class="border-yellow-400">
      <CardContent class="pt-4">
        <p class="text-yellow-700 dark:text-yellow-300 font-medium">
          Hot reload is disabled on this instance (<code>BATLEHUB_DISABLE_HOT_RELOAD=1</code>).
          Config changes require a server restart.
        </p>
      </CardContent>
    </Card>

    <!-- Feedback -->
    <div v-if="successMsg" class="rounded-md bg-green-50 dark:bg-green-950 border border-green-400 px-4 py-2 text-green-800 dark:text-green-200 text-sm">
      {{ successMsg }}
    </div>
    <div v-if="errorMsg" class="rounded-md bg-red-50 dark:bg-red-950 border border-red-400 px-4 py-2 text-red-800 dark:text-red-200 text-sm">
      {{ errorMsg }}
    </div>

    <!-- Pending Reload Card -->
    <Card v-if="hotReloadEnabled !== false">
      <CardHeader>
        <CardTitle>Pending Reload</CardTitle>
      </CardHeader>
      <CardContent>
        <div v-if="loadingPending && !pendingReload" class="text-sm text-muted-foreground">Loading…</div>
        <div v-else-if="!pendingReload" class="text-sm text-muted-foreground">
          No pending reload. The file watcher will populate this when a config change is detected.
        </div>
        <div v-else class="space-y-3">
          <div class="flex gap-4 text-sm">
            <span><strong>Source:</strong> {{ pendingReload.source }}</span>
            <span><strong>Created:</strong> {{ new Date(pendingReload.created_at).toLocaleString() }}</span>
            <span><strong>Expires in:</strong> {{ expiresIn }}</span>
          </div>
          <div class="flex gap-2 flex-wrap">
            <Badge v-for="r in pendingReload.diff.added_registries" :key="r" class="bg-green-100 text-green-800">+{{ r }}</Badge>
            <Badge v-for="r in pendingReload.diff.removed_registries" :key="r" class="bg-red-100 text-red-800">-{{ r }}</Badge>
            <Badge v-for="r in pendingReload.diff.changed_registries" :key="r.name" class="bg-yellow-100 text-yellow-800">~{{ r.name }}</Badge>
            <Badge v-if="pendingReload.diff.limits_changed" class="bg-purple-100 text-purple-800">limits changed</Badge>
          </div>
          <div class="flex gap-2">
            <Button size="sm" :disabled="loadingApply" @click="applyPending">
              {{ loadingApply ? "Applying…" : "Apply" }}
            </Button>
            <Button size="sm" variant="outline" :disabled="loadingDiscard" @click="discardPending">
              {{ loadingDiscard ? "Discarding…" : "Discard" }}
            </Button>
          </div>
        </div>
      </CardContent>
    </Card>

    <!-- Force Reload Card -->
    <Card v-if="hotReloadEnabled !== false">
      <CardHeader>
        <CardTitle>Force Reload Now</CardTitle>
      </CardHeader>
      <CardContent class="space-y-2">
        <p class="text-sm text-muted-foreground">
          Re-reads the config file, validates it, and applies it immediately — no confirmation step.
        </p>
        <Button :disabled="loadingForce" @click="forceReload">
          {{ loadingForce ? "Reloading…" : "Reload Now" }}
        </Button>
      </CardContent>
    </Card>

    <!-- Global Banner Card -->
    <Card>
      <CardHeader>
        <CardTitle>Global Banner</CardTitle>
      </CardHeader>
      <CardContent class="space-y-4">
        <div v-if="banner" class="rounded-md border px-3 py-2 text-sm">
          <strong>Current:</strong> [{{ banner.level }}] {{ banner.message }}
          <span class="text-muted-foreground ml-2">— set by {{ banner.set_by }}</span>
        </div>
        <div v-else class="text-sm text-muted-foreground">No banner currently set.</div>
        <div class="flex gap-2 items-end flex-wrap">
          <div class="flex-1 min-w-[16rem] space-y-1">
            <Label>Message</Label>
            <Input v-model="bannerMessage" placeholder="Maintenance in progress…" />
          </div>
          <div class="space-y-1">
            <Label>Level</Label>
            <select v-model="bannerLevel" class="border rounded px-2 py-2 text-sm bg-background">
              <option value="info">Info</option>
              <option value="warning">Warning</option>
              <option value="error">Error</option>
            </select>
          </div>
          <Button :disabled="loadingSetBanner || !bannerMessage.trim()" @click="setBannerAction">
            {{ loadingSetBanner ? "Setting…" : "Set Banner" }}
          </Button>
          <Button variant="outline" :disabled="loadingClearBanner || !banner" @click="clearBannerAction">
            {{ loadingClearBanner ? "Clearing…" : "Clear Banner" }}
          </Button>
        </div>
      </CardContent>
    </Card>

    <!-- Change History -->
    <Card>
      <CardHeader>
        <CardTitle>Change History</CardTitle>
      </CardHeader>
      <CardContent>
        <div v-if="loadingHistory" class="text-sm text-muted-foreground">Loading…</div>
        <div v-else-if="changeHistory.length === 0" class="text-sm text-muted-foreground">No changes recorded yet.</div>
        <table v-else class="w-full text-sm">
          <thead>
            <tr class="text-left border-b">
              <th class="pb-2 pr-4">Date</th>
              <th class="pb-2 pr-4">By</th>
              <th class="pb-2 pr-4">Status</th>
              <th class="pb-2">Summary</th>
            </tr>
          </thead>
          <tbody>
            <template v-for="row in changeHistory" :key="row.id">
              <tr
                class="border-b cursor-pointer hover:bg-muted/30"
                @click="expandedRow = expandedRow === row.id ? null : row.id"
              >
                <td class="py-2 pr-4">{{ new Date(row.triggered_at).toLocaleString() }}</td>
                <td class="py-2 pr-4">{{ row.triggered_by }}</td>
                <td class="py-2 pr-4">
                  <Badge :class="row.status === 'applied' ? 'bg-green-100 text-green-800' : 'bg-red-100 text-red-800'">
                    {{ row.status }}
                  </Badge>
                </td>
                <td class="py-2">{{ row.summary }}</td>
              </tr>
              <tr v-if="expandedRow === row.id">
                <td colspan="4" class="pb-3">
                  <pre class="bg-muted text-xs p-2 rounded overflow-x-auto">{{ JSON.stringify(row.diff, null, 2) }}</pre>
                </td>
              </tr>
            </template>
          </tbody>
        </table>
      </CardContent>
    </Card>
  </div>
</template>
