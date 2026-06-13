<script setup lang="ts">
import { computed } from "vue";
import { useRoute, useRouter } from "vue-router";
import { packageDetail } from "@/client/sdk.gen";
import type { PackageDetailResponse } from "@/client/types.gen";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { useUpstreamUrl } from "@/composables/useUpstreamUrl";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import PackageHeaderCard from "@/components/PackageHeaderCard.vue";
import PackageVersionsTable from "@/components/admin/PackageVersionsTable.vue";
import PackageBetaChannel from "@/components/admin/PackageBetaChannel.vue";
import PackageVisibility from "@/components/admin/PackageVisibility.vue";
import PackageEventsTable from "@/components/admin/PackageEventsTable.vue";

const route = useRoute();
const router = useRouter();
const { token } = useAuth();

const registry = computed(() => String(route.query.registry ?? ""));
const name = computed(() => String(route.query.name ?? ""));

const upstreamUrl = useUpstreamUrl(registry, name, token);

const { data, error, loading, reload } = useApi<PackageDetailResponse>(
  () =>
    packageDetail({ query: { registry: registry.value, name: name.value } }) as Promise<{
      data?: unknown;
      error?: unknown;
    }>,
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
      <PackageHeaderCard :name="data.name" :upstream-url="upstreamUrl">
        <template #badges>
          <div>
            <span class="text-muted-foreground w-28 inline-block">Registry</span>
            <Badge variant="outline">{{ data.registry }}</Badge>
          </div>
        </template>
        <div>
          <span class="text-muted-foreground w-28 inline-block">Versions</span
          >{{ data.versions.length }}
        </div>
      </PackageHeaderCard>

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
