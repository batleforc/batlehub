<script setup lang="ts">
import { AlertTriangle, Info, XCircle } from "@lucide/vue";
import { useBanner } from "@/composables/useBanner";

const { banner } = useBanner();
</script>

<template>
  <div
    v-if="banner"
    :class="[
      'border-t px-4 py-1.5 flex items-center gap-2 text-sm font-mono font-medium',
      banner.level === 'error'
        ? 'border-destructive/40 text-destructive'
        : banner.level === 'warning'
          ? 'border-copper/40 text-copper'
          : 'border-foreground/40 text-foreground',
    ]"
  >
    <XCircle v-if="banner.level === 'error'" class="h-3.5 w-3.5 shrink-0" />
    <AlertTriangle v-else-if="banner.level === 'warning'" class="h-3.5 w-3.5 shrink-0" />
    <Info v-else class="h-3.5 w-3.5 shrink-0" />
    <span>{{ banner.message }}</span>
  </div>
</template>
