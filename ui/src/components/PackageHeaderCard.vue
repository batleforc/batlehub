<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";

const { t } = useI18n();

defineProps<{
  name: string;
  upstreamUrl: string | null;
}>();
</script>

<template>
  <Card>
    <CardHeader>
      <CardTitle class="text-xl font-mono">{{ name }}</CardTitle>
    </CardHeader>
    <CardContent class="space-y-2 text-sm">
      <slot name="badges" />
      <slot name="before-upstream" />
      <div>
        <span class="text-muted-foreground w-28 inline-block">{{ t("common.upstream") }}</span>
        <a
          v-if="upstreamUrl"
          :href="upstreamUrl"
          target="_blank"
          rel="noopener noreferrer"
          class="text-primary underline-offset-2 hover:underline font-mono text-xs"
          >{{ upstreamUrl }}</a
        >
        <span v-else class="text-muted-foreground">—</span>
      </div>
      <slot />
    </CardContent>
  </Card>
</template>
