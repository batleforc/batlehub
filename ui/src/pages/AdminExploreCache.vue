<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useAuth } from "@/composables/useAuth";
import { Card, CardHeader, CardTitle, CardContent, CardDescription } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";

const { token } = useAuth();
const API_BASE = import.meta.env.VITE_API_BASE_URL ?? "";

// ── State ─────────────────────────────────────────────────────────────────────

const registries = ref<string[]>([]);
const selectedRegistry = ref<string>("");

const loadingAll = ref(false);
const loadingRegistry = ref(false);
const successMsg = ref<string | null>(null);
const errorMsg = ref<string | null>(null);

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

function notify(msg: string, isError = false) {
  successMsg.value = null;
  errorMsg.value = null;
  if (isError) errorMsg.value = msg;
  else successMsg.value = msg;
}

// ── Registry list ─────────────────────────────────────────────────────────────

async function fetchRegistries() {
  try {
    const resp = await apiFetch("/api/v1/admin/health");
    const list = (await resp.json()) as { registry: string }[];
    registries.value = list.map((r) => r.registry).sort();
    if (registries.value.length > 0) selectedRegistry.value = registries.value[0];
  } catch {
    // non-fatal — the UI still works with a text input fallback
  }
}

// ── Actions ───────────────────────────────────────────────────────────────────

async function invalidateAll() {
  loadingAll.value = true;
  successMsg.value = null;
  errorMsg.value = null;
  try {
    await apiFetch("/api/v1/admin/explore/invalidate", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({}),
    });
    notify("Entire explore cache cleared. Next requests will hit the database.");
  } catch (e) {
    notify(e instanceof Error ? e.message : String(e), true);
  } finally {
    loadingAll.value = false;
  }
}

async function invalidateRegistry() {
  if (!selectedRegistry.value.trim()) return;
  loadingRegistry.value = true;
  successMsg.value = null;
  errorMsg.value = null;
  try {
    await apiFetch("/api/v1/admin/explore/invalidate", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ registry: selectedRegistry.value.trim() }),
    });
    notify(`Explore cache cleared for registry "${selectedRegistry.value}".`);
  } catch (e) {
    notify(e instanceof Error ? e.message : String(e), true);
  } finally {
    loadingRegistry.value = false;
  }
}

onMounted(fetchRegistries);
</script>

<template>
  <div class="space-y-6">
    <div>
      <h1 class="text-2xl font-bold">Explore Cache</h1>
      <p class="text-sm text-muted-foreground mt-1">
        The package explorer caches database query results for
        <Badge variant="outline" class="font-mono text-xs">10 min</Badge>
        to avoid expensive scans on large registries.
        Stale entries are kept and served if the database becomes unreachable.
      </p>
    </div>

    <!-- Feedback -->
    <div
      v-if="successMsg"
      class="rounded-md bg-green-50 dark:bg-green-950 border border-green-400 px-4 py-2 text-green-800 dark:text-green-200 text-sm"
    >
      {{ successMsg }}
    </div>
    <div
      v-if="errorMsg"
      class="rounded-md bg-red-50 dark:bg-red-950 border border-red-400 px-4 py-2 text-red-800 dark:text-red-200 text-sm"
    >
      {{ errorMsg }}
    </div>

    <!-- Per-registry invalidation -->
    <Card>
      <CardHeader>
        <CardTitle>Invalidate by Registry</CardTitle>
        <CardDescription>
          Clears only the entries belonging to one registry. Use this after a manual data fix or
          forced re-index without triggering a full publish.
        </CardDescription>
      </CardHeader>
      <CardContent class="space-y-3">
        <div class="flex gap-2 flex-wrap items-end">
          <!-- Populated select when health endpoint succeeds -->
          <div v-if="registries.length > 0" class="flex flex-col gap-1">
            <label class="text-xs text-muted-foreground font-medium">Registry</label>
            <select
              v-model="selectedRegistry"
              class="border rounded px-2 py-2 text-sm bg-background min-w-[10rem]"
            >
              <option v-for="r in registries" :key="r" :value="r">{{ r }}</option>
            </select>
          </div>
          <!-- Fallback text input -->
          <div v-else class="flex flex-col gap-1">
            <label class="text-xs text-muted-foreground font-medium">Registry name</label>
            <input
              v-model="selectedRegistry"
              placeholder="e.g. npm"
              class="border rounded px-2 py-2 text-sm bg-background min-w-[10rem]"
            />
          </div>

          <Button
            :disabled="loadingRegistry || !selectedRegistry.trim()"
            @click="invalidateRegistry"
          >
            {{ loadingRegistry ? "Invalidating…" : "Invalidate Registry" }}
          </Button>
        </div>

        <p class="text-xs text-muted-foreground">
          Cache is also invalidated automatically when a package is published to this registry.
        </p>
      </CardContent>
    </Card>

    <!-- Full cache flush -->
    <Card>
      <CardHeader>
        <CardTitle>Invalidate Entire Cache</CardTitle>
        <CardDescription>
          Forces every explore endpoint to re-query the database on the next request.
          Use after bulk data imports or registry restructuring.
        </CardDescription>
      </CardHeader>
      <CardContent class="space-y-3">
        <Button variant="destructive" :disabled="loadingAll" @click="invalidateAll">
          {{ loadingAll ? "Clearing…" : "Invalidate All Registries" }}
        </Button>
        <p class="text-xs text-muted-foreground">
          The cache repopulates automatically on the next request — no downtime.
        </p>
      </CardContent>
    </Card>

    <!-- Behaviour reference -->
    <Card>
      <CardHeader>
        <CardTitle>How the Cache Works</CardTitle>
      </CardHeader>
      <CardContent>
        <ul class="text-sm space-y-1.5 text-muted-foreground list-disc list-inside">
          <li>Results are cached per query (registry filter, search term, sort, page).</li>
          <li>TTL is 10 minutes. Expired entries are served stale if the database is unreachable.</li>
          <li>
            When the database is unreachable and no cached entry exists, the response includes
            <code class="text-xs bg-muted px-1 rounded">upstream_unavailable: true</code>
            so the UI can display a warning.
          </li>
          <li>Publishing a package invalidates all cached entries for that registry automatically.</li>
          <li>
            The cache is in-memory and per-instance — a server restart or horizontal scale-out
            starts with an empty cache.
          </li>
        </ul>
      </CardContent>
    </Card>
  </div>
</template>
