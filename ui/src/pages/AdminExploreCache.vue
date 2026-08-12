<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { ref, onMounted } from "vue";
import { registryHealth, invalidateExploreCache } from "@/client/sdk.gen";
import SectionTabs from "@/components/admin/SectionTabs.vue";
import { OPERATIONS_TABS } from "@/config/adminSections";
import { PageHeader } from "@/components/ui/page-header";
import { Card, CardHeader, CardTitle, CardContent, CardDescription } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";

const { t } = useI18n();

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
      registries.value = (data as { registry: string }[])
        .map((r) => r.registry)
        .sort((a, b) => a.localeCompare(b));
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
    const { error: apiErr } = await invalidateExploreCache({
      body: { registry: selectedRegistry.value.trim() },
    });
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
    <SectionTabs :tabs="OPERATIONS_TABS" />
    <PageHeader :title="t('adminExploreCache.exploreCache')">
      <template #description>
        {{ t("adminExploreCache.cachesQueryResultsFor") }}
        <Badge variant="outline" class="font-mono text-xs">{{
          t("adminExploreCache.10Min")
        }}</Badge>
        {{ t("adminExploreCache.toAvoidExpensiveScans") }}
      </template>
    </PageHeader>

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
        <CardTitle>{{ t("adminExploreCache.invalidateByRegistry") }}</CardTitle>
        <CardDescription>{{ t("adminExploreCache.clearsOnlyTheEntries") }}</CardDescription>
      </CardHeader>
      <CardContent class="space-y-3">
        <div class="flex gap-2 flex-wrap items-end">
          <!-- Populated select when health endpoint succeeds -->
          <div v-if="registries.length > 0" class="flex flex-col gap-1">
            <label
              for="explore-cache-registry-select"
              class="text-xs text-muted-foreground font-medium"
              >{{ t("common.registry") }}</label
            >
            <select
              id="explore-cache-registry-select"
              v-model="selectedRegistry"
              class="border border-input rounded-sm px-2 py-2 font-mono text-sm bg-background min-w-[10rem] focus:outline-none focus:ring-2 focus:ring-ring"
            >
              <option v-for="r in registries" :key="r" :value="r">{{ r }}</option>
            </select>
          </div>
          <!-- Fallback text input -->
          <div v-else class="flex flex-col gap-1">
            <label
              for="explore-cache-registry-input"
              class="text-xs text-muted-foreground font-medium"
              >{{ t("adminExploreCache.registryName") }}</label
            >
            <input
              id="explore-cache-registry-input"
              v-model="selectedRegistry"
              placeholder="e.g. npm"
              class="border border-input rounded-sm px-2 py-2 font-mono text-sm bg-background min-w-[10rem] focus:outline-none focus:ring-2 focus:ring-ring"
            />
          </div>

          <Button
            :disabled="loadingRegistry || !selectedRegistry.trim()"
            @click="invalidateRegistry"
          >
            {{
              loadingRegistry
                ? t("adminExploreCache.invalidating")
                : t("adminExploreCache.invalidateRegistry")
            }}
          </Button>
        </div>

        <p class="text-xs text-muted-foreground">
          {{ t("adminExploreCache.cacheIsAlsoInvalidated") }}
        </p>
      </CardContent>
    </Card>

    <!-- Full cache flush -->
    <Card>
      <CardHeader>
        <CardTitle>{{ t("adminExploreCache.invalidateEntireCache") }}</CardTitle>
        <CardDescription>{{ t("adminExploreCache.forcesEveryExploreEndpoint") }}</CardDescription>
      </CardHeader>
      <CardContent class="space-y-3">
        <Button variant="destructive" :disabled="loadingAll" @click="invalidateAll">
          {{
            loadingAll
              ? t("adminConfigReload.clearing")
              : t("adminExploreCache.invalidateAllRegistries")
          }}
        </Button>
        <p class="text-xs text-muted-foreground">
          {{ t("adminExploreCache.theCacheRepopulatesAutomatically") }}
        </p>
      </CardContent>
    </Card>

    <!-- Behaviour reference -->
    <Card>
      <CardHeader>
        <CardTitle>{{ t("adminExploreCache.howTheCacheWorks") }}</CardTitle>
      </CardHeader>
      <CardContent>
        <ul class="text-sm space-y-1.5 text-muted-foreground list-disc list-inside">
          <li>{{ t("adminExploreCache.resultsAreCachedPer") }}</li>
          <li>{{ t("adminExploreCache.ttlIs10Minutes") }}</li>
          <li>
            <i18n-t keypath="adminExploreCache.upstreamUnavailableFlag" tag="span">
              <template #flag
                ><code class="text-xs bg-muted px-1 rounded">{{
                  t("adminExploreCache.upstream_unavailableTrue")
                }}</code></template
              >
            </i18n-t>
          </li>
          <li>{{ t("adminExploreCache.publishingAPackageInvalidates") }}</li>
          <li>{{ t("adminExploreCache.theCacheIsIn") }}</li>
        </ul>
      </CardContent>
    </Card>
  </div>
</template>
