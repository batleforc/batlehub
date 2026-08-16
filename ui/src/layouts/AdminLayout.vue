<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { RouterView, RouterLink, useRoute } from "vue-router";
import { ShieldCheck } from "@lucide/vue";
import { ADMIN_SIDEBAR } from "@/config/adminSections";

const { t } = useI18n();
const route = useRoute();

function isActive(to: string) {
  return route.path === to || route.path.startsWith(to + "/");
}
</script>

<template>
  <!--
    Column on mobile, row from `md` up.

    It was a row at every width, and the mobile tab strip below is `w-full` —
    so under `md` the strip and the content sat *side by side* in the same flex
    row and the document came out 496–705px wide on a 390px viewport. Every one
    of the fifteen admin pages scrolled sideways, and none of the design gates
    could see it: the rendered detector runs 390x844 but only over the
    unauthenticated routes, and the authenticated gate covers `/admin/*` at
    1440x900 only. `design-authed.mjs` now measures both widths (RFC 0004 §10).
  -->
  <div class="flex flex-col md:flex-row md:gap-6 min-h-[calc(100vh-3.5rem-1px)]">
    <!-- Sidebar (desktop) -->
    <aside class="hidden md:flex flex-col w-52 shrink-0 border-r border-border/60 pr-4 pt-2">
      <div
        class="flex items-center gap-2 px-3 py-2 mb-2 font-mono text-xs font-semibold uppercase tracking-wider text-copper"
      >
        <ShieldCheck class="h-3.5 w-3.5" />
        {{ t("common.admin") }}
      </div>
      <nav class="flex flex-col gap-0.5">
        <RouterLink
          v-for="link in ADMIN_SIDEBAR"
          :key="link.to"
          :to="link.to"
          :class="[
            'flex items-center gap-2.5 px-3 py-2 rounded-sm font-mono text-sm transition-colors',
            isActive(link.to)
              ? 'bg-accent text-accent-foreground font-semibold border-l-2 border-primary'
              : 'text-muted-foreground hover:bg-accent/60 hover:text-accent-foreground',
          ]"
        >
          <component :is="link.icon" class="h-4 w-4 shrink-0" />
          {{ t(link.label) }}
        </RouterLink>
      </nav>
    </aside>

    <!-- Mobile: horizontal tab strip -->
    <div class="md:hidden -mx-4 px-4 border-b border-border/60 mb-4 w-full flex flex-col">
      <div class="flex items-center gap-1 pb-1 overflow-x-auto">
        <span class="flex items-center gap-1 font-mono text-xs text-copper mr-2 shrink-0">
          <ShieldCheck class="h-3 w-3" /> {{ t("common.admin") }}
        </span>
        <RouterLink
          v-for="link in ADMIN_SIDEBAR"
          :key="link.to"
          :to="link.to"
          :class="[
            'flex items-center gap-1.5 px-3 py-1.5 rounded-sm font-mono text-sm whitespace-nowrap transition-colors shrink-0',
            isActive(link.to)
              ? 'bg-accent text-accent-foreground font-semibold'
              : 'text-muted-foreground hover:bg-accent/60 hover:text-accent-foreground',
          ]"
        >
          <component :is="link.icon" class="h-3.5 w-3.5 shrink-0" />
          {{ t(link.label) }}
        </RouterLink>
      </div>
    </div>

    <!-- Content -->
    <div class="flex-1 min-w-0 pt-2">
      <RouterView />
    </div>
  </div>
</template>
