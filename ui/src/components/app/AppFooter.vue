<script setup lang="ts">
import { onMounted, ref } from "vue";
import { API_BASE_URL, REPORT_BUG_URL, REPORT_SECURITY_URL } from "@/config";

const appVersion = ref<string | null>(null);

onMounted(async () => {
  try {
    const res = await fetch(`${API_BASE_URL}/healthz`);
    if (res.ok) {
      const data = (await res.json()) as { version?: string };
      appVersion.value = data.version ?? null;
    }
  } catch {
    // Best-effort only — the footer just omits the version if unreachable.
  }
});
</script>

<template>
  <footer class="border-t border-border/60 mt-8">
    <div
      class="container mx-auto flex flex-wrap items-center justify-between gap-2 px-4 py-3 text-xs font-mono text-muted-foreground"
    >
      <span v-if="appVersion">BatleHub v{{ appVersion }}</span>
      <span v-else />
      <div class="flex items-center gap-3">
        <a
          :href="REPORT_BUG_URL"
          target="_blank"
          rel="noopener noreferrer"
          class="hover:text-accent-foreground transition-colors"
        >
          Report a bug
        </a>
        <a
          :href="REPORT_SECURITY_URL"
          target="_blank"
          rel="noopener noreferrer"
          class="hover:text-accent-foreground transition-colors"
        >
          Report a security issue
        </a>
      </div>
    </div>
  </footer>
</template>
