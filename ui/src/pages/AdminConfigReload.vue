<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import {
  discardPendingReload,
  applyPendingReload,
  reloadConfig,
  listConfigChanges,
  setBanner,
  clearBanner,
} from "@/client/sdk.gen";
import type { PendingReloadSnapshot, ConfigChangeRow } from "@/client/types.gen";
import { useAuthFetch } from "@/composables/useAuthFetch";
import { useBanner } from "@/composables/useBanner";
import { API_BASE_URL } from "@/config";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

const { authFetch } = useAuthFetch();
const { banner } = useBanner();

// ── State ─────────────────────────────────────────────────────────────────────

const hotReloadEnabled = ref<boolean | null>(null);
const pendingReload = ref<PendingReloadSnapshot | null>(null);
const changeHistory = ref<ConfigChangeRow[]>([]);

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

function sdkErrMsg(err: unknown): string {
  if (!err) return "API error";
  const e = err as { message?: string };
  return e.message ?? String(err);
}

async function fetchPending() {
  loadingPending.value = true;
  try {
    // Use raw fetch to distinguish 404 (no pending) from 503 (hot reload disabled).
    const resp = await authFetch(`${API_BASE_URL}/api/v1/admin/config/pending`);
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
    const { data } = await listConfigChanges({ query: { per_page: 20 } });
    changeHistory.value = (data as { items?: ConfigChangeRow[] })?.items ?? [];
  } catch {
    // non-critical — history list will remain empty
  } finally {
    loadingHistory.value = false;
  }
}

async function forceReload() {
  loadingForce.value = true;
  errorMsg.value = null;
  successMsg.value = null;
  try {
    const { data, error: apiErr } = await reloadConfig();
    if (apiErr) throw new Error(sdkErrMsg(apiErr));
    const diff = (data as { diff?: { added_registries: string[]; removed_registries: string[] } })
      ?.diff;
    successMsg.value = `Reloaded: +${diff?.added_registries.length ?? 0} -${diff?.removed_registries.length ?? 0} registries`;
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
    const { data, error: apiErr } = await applyPendingReload();
    if (apiErr) throw new Error(sdkErrMsg(apiErr));
    const diff = (data as { diff?: { added_registries: string[]; removed_registries: string[] } })
      ?.diff;
    successMsg.value = `Applied: +${diff?.added_registries.length ?? 0} -${diff?.removed_registries.length ?? 0} registries`;
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
    const { error: apiErr } = await discardPendingReload();
    if (apiErr) throw new Error(sdkErrMsg(apiErr));
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
    const { error: apiErr } = await setBanner({
      body: { message: bannerMessage.value, level: bannerLevel.value },
    });
    if (apiErr) throw new Error(sdkErrMsg(apiErr));
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
    const { error: apiErr } = await clearBanner();
    if (apiErr) throw new Error(sdkErrMsg(apiErr));
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
onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer);
});
</script>

<template>
  <div class="space-y-6">
    <h1 class="text-2xl font-bold">Config Reload</h1>

    <!-- Status: hot reload disabled -->
    <Card v-if="hotReloadEnabled === false" class="border-yellow-400">
      <CardContent class="pt-4">
        <p class="text-copper font-medium">
          Hot reload is disabled on this instance (<code>BATLEHUB_DISABLE_HOT_RELOAD=1</code>).
          Config changes require a server restart.
        </p>
      </CardContent>
    </Card>

    <!-- Feedback -->
    <div
      v-if="successMsg"
      class="rounded-sm bg-primary/10 border border-primary/30 px-4 py-2 text-primary text-sm"
    >
      {{ successMsg }}
    </div>
    <div
      v-if="errorMsg"
      class="rounded-sm bg-destructive/10 border border-destructive/30 px-4 py-2 text-destructive text-sm"
    >
      {{ errorMsg }}
    </div>

    <!-- Pending Reload Card -->
    <Card v-if="hotReloadEnabled !== false">
      <CardHeader>
        <CardTitle>Pending Reload</CardTitle>
      </CardHeader>
      <CardContent>
        <div v-if="loadingPending && !pendingReload" class="text-sm text-muted-foreground">
          Loading…
        </div>
        <div v-else-if="!pendingReload" class="text-sm text-muted-foreground">
          No pending reload. The file watcher will populate this when a config change is detected.
        </div>
        <div v-else class="space-y-3">
          <div class="flex gap-4 text-sm">
            <span><strong>Source:</strong> {{ pendingReload.source }}</span>
            <span
              ><strong>Created:</strong>
              {{ new Date(pendingReload.created_at).toLocaleString() }}</span
            >
            <span><strong>Expires in:</strong> {{ expiresIn }}</span>
          </div>
          <div class="flex gap-2 flex-wrap">
            <Badge
              v-for="r in pendingReload.diff.added_registries"
              :key="r"
              class="bg-primary/10 text-primary"
              >+{{ r }}</Badge
            >
            <Badge
              v-for="r in pendingReload.diff.removed_registries"
              :key="r"
              class="bg-destructive/10 text-destructive"
              >-{{ r }}</Badge
            >
            <Badge
              v-for="r in pendingReload.diff.changed_registries"
              :key="r.name"
              class="bg-copper/10 text-copper"
              >~{{ r.name }}</Badge
            >
            <Badge v-if="pendingReload.diff.limits_changed" class="bg-purple-100 text-purple-800"
              >limits changed</Badge
            >
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
        <div v-if="banner" class="rounded-sm border px-3 py-2 text-sm">
          <strong>Current:</strong> [{{ banner.level }}] {{ banner.message }}
          <span class="text-muted-foreground ml-2">— set by {{ banner.set_by }}</span>
        </div>
        <div v-else class="text-sm text-muted-foreground">No banner currently set.</div>
        <div class="flex gap-2 items-end flex-wrap">
          <div class="flex-1 min-w-[16rem] space-y-1">
            <Label for="banner-message">Message</Label>
            <Input
              id="banner-message"
              v-model="bannerMessage"
              placeholder="Maintenance in progress…"
            />
          </div>
          <div class="space-y-1">
            <Label for="banner-level">Level</Label>
            <select
              id="banner-level"
              v-model="bannerLevel"
              class="border border-input rounded-sm px-2 py-2 font-mono text-sm bg-background focus:outline-none focus:ring-2 focus:ring-ring"
            >
              <option value="info">Info</option>
              <option value="warning">Warning</option>
              <option value="error">Error</option>
            </select>
          </div>
          <Button :disabled="loadingSetBanner || !bannerMessage.trim()" @click="setBannerAction">
            {{ loadingSetBanner ? "Setting…" : "Set Banner" }}
          </Button>
          <Button
            variant="outline"
            :disabled="loadingClearBanner || !banner"
            @click="clearBannerAction"
          >
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
        <div v-else-if="changeHistory.length === 0" class="text-sm text-muted-foreground">
          No changes recorded yet.
        </div>
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
                  <Badge
                    :class="
                      row.status === 'applied'
                        ? 'bg-green-100 text-primary'
                        : 'bg-destructive/10 text-destructive'
                    "
                  >
                    {{ row.status }}
                  </Badge>
                </td>
                <td class="py-2">{{ row.summary }}</td>
              </tr>
              <tr v-if="expandedRow === row.id">
                <td colspan="4" class="pb-3">
                  <pre class="bg-muted text-xs p-2 rounded overflow-x-auto">{{
                    JSON.stringify(row.diff, null, 2)
                  }}</pre>
                </td>
              </tr>
            </template>
          </tbody>
        </table>
      </CardContent>
    </Card>
  </div>
</template>
