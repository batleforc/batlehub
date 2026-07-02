<script setup lang="ts">
import { Badge } from "@/components/ui/badge";
import { CopyButton } from "@/components/ui/copy-button";
import type { ProxyPath } from "@/config/registryPathFields";

const props = defineProps<{
  paths: ProxyPath[];
  baseUrl: string;
}>();

function fullUrl(path: string): string {
  return `${props.baseUrl}${path}`;
}
</script>

<template>
  <div v-if="paths.length" class="space-y-2">
    <h2 class="text-sm font-medium text-muted-foreground uppercase tracking-wide">Proxy paths</h2>
    <div class="rounded-sm border divide-y">
      <div
        v-for="entry in paths"
        :key="entry.url"
        class="flex items-center gap-3 px-4 py-3"
        :class="entry.available ? '' : 'opacity-40'"
      >
        <span class="w-44 shrink-0 text-xs text-muted-foreground">{{ entry.label }}</span>
        <code class="flex-1 text-xs font-mono truncate" :title="fullUrl(entry.url)">
          {{ fullUrl(entry.url) }}
        </code>
        <CopyButton
          v-if="entry.available"
          :text="fullUrl(entry.url)"
          class="shrink-0 h-7 px-2 text-xs"
        />
        <Badge v-else variant="outline" class="shrink-0 text-xs"> needs more fields </Badge>
      </div>
    </div>
  </div>

  <p v-else class="text-sm text-muted-foreground text-center py-4">
    Fill in the fields above to see the proxy paths.
  </p>
</template>
