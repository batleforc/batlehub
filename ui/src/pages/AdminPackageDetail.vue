<script setup lang="ts">
import { computed } from "vue";
import { useRoute, useRouter } from "vue-router";
import { listRegistries, packageDetail } from "@/client/sdk.gen";
import type { RegistryInfo, PackageDetailResponse } from "@/client/types.gen";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import PackageVersionsTable from "@/components/admin/PackageVersionsTable.vue";
import PackageBetaChannel from "@/components/admin/PackageBetaChannel.vue";
import PackageVisibility from "@/components/admin/PackageVisibility.vue";
import PackageEventsTable from "@/components/admin/PackageEventsTable.vue";

const route = useRoute();
const router = useRouter();
const { token } = useAuth();

const registry = computed(() => String(route.query.registry ?? ""));
const name = computed(() => String(route.query.name ?? ""));

const { data: registriesList } = useApi<RegistryInfo[]>(
  () => listRegistries() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

const registryType = computed(
  () => registriesList.value?.find(r => r.name === registry.value)?.type ?? null,
);

const upstreamUrl = computed(() => {
  if (!registry.value || !name.value) return null;
  switch (registryType.value) {
    case "github": return `https://github.com/${name.value}`;
    case "npm":    return `https://www.npmjs.com/package/${name.value}`;
    case "cargo":  return `https://crates.io/crates/${name.value}`;
    default:       return null;
  }
});

const { data, error, loading, reload } = useApi<PackageDetailResponse>(
  () => packageDetail({ query: { registry: registry.value, name: name.value } }) as Promise<{ data?: unknown; error?: unknown }>,
  [token, registry, name],
);
</script>

<template>
  <div class="space-y-4">
    <!-- Back -->
    <div class="flex items-center gap-3">
      <Button variant="ghost" size="sm" @click="router.back()">← Back</Button>
      <span class="text-muted-foreground text-sm">/</span>
      <span class="font-mono text-sm">{{ registry }}/{{ name }}</span>
    </div>

    <p v-if="loading" class="text-sm text-muted-foreground">Loading…</p>
    <p v-else-if="error" class="text-sm text-destructive">{{ error }}</p>

    <template v-else-if="data">
      <!-- Header card -->
      <Card>
        <CardHeader>
          <CardTitle class="text-xl font-mono">{{ data.name }}</CardTitle>
        </CardHeader>
        <CardContent class="space-y-1 text-sm">
          <div>
            <span class="text-muted-foreground w-28 inline-block">Registry</span>
            <Badge variant="outline">{{ data.registry }}</Badge>
          </div>
          <div>
            <span class="text-muted-foreground w-28 inline-block">Upstream</span>
            <a
              v-if="upstreamUrl"
              :href="upstreamUrl"
              target="_blank"
              rel="noopener noreferrer"
              class="text-primary underline-offset-2 hover:underline font-mono text-xs"
            >{{ upstreamUrl }}</a>
            <span v-else class="text-muted-foreground">—</span>
          </div>
          <div>
            <span class="text-muted-foreground w-28 inline-block">Versions</span>{{ data.versions.length }}
          </div>
        </CardContent>
      </Card>

      <PackageVersionsTable
        :registry="registry"
        :name="name"
        :versions="data.versions"
        @reload="reload"
      />

      <PackageBetaChannel :registry="registry" />

      <PackageVisibility :registry="registry" :name="name" />

      <PackageEventsTable :events="data.recent_events" />
    </template>
  </div>
</template>
