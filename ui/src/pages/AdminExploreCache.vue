<script setup lang="ts">
import { ref, onMounted } from "vue";
import { registryHealth, invalidateExploreCache } from "@/client/sdk.gen";
import { Card, CardHeader, CardTitle, CardContent, CardDescription } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";

// ── State ─────────────────────────────────────────────────────────────────────

const registries = ref<string[]>([]);
const selectedRegistry = ref<string>("");

const loadingAll = ref(false);
const loadingRegistry = ref(false);
const successMsg = ref<string | null>(null);
const errorMsg = ref<string | null>(null);

function notify(msg: string, isError = false) {
  successMsg.value = null;
  errorMsg.value = null;
  if (isError) errorMsg.value = msg;
  else successMsg.value = msg;
}

// ── Registry list ─────────────────────────────────────────────────────────────

async function fetchRegistries() {
  try {
    const { data } = await registryHealth();
    if (data) {
      registries.value = (data as { registry: string }[]).map((r) => r.registry).sort();
      if (registries.value.length > 0) selectedRegistry.value = registries.value[0];
    }
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
    const { error: apiErr } = await invalidateExploreCache({ body: {} });
    if (apiErr) throw new Error((apiErr as { message?: string })?.message ?? "API error");
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
    const { error: apiErr } = await invalidateExploreCache({ body: { registry: selectedRegistry.value.trim() } });
    if (apiErr) throw new Error((apiErr as { message?: string })?.message ?? "API error");
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
              class="border border-input rounded-sm px-2 py-2 font-mono text-sm bg-background min-w-[10rem] focus:outline-none focus:ring-2 focus:ring-ring"
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
              class="border border-input rounded-sm px-2 py-2 font-mono text-sm bg-background min-w-[10rem] focus:outline-none focus:ring-2 focus:ring-ring"
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
