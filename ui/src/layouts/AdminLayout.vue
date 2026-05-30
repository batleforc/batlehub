<script setup lang="ts">
import { RouterView, RouterLink, useRoute } from "vue-router";
import {
  ShieldCheck,
  Package,
  Upload,
  ScrollText,
  HeartPulse,
  Shield,
  FlaskConical,
  FolderKey,
} from "@lucide/vue";

const route = useRoute();

const adminLinks = [
  { to: "/admin/packages",     label: "Packages",     icon: Package },
  { to: "/admin/bulk",         label: "Bulk Import",  icon: Upload },
  { to: "/admin/audit-log",    label: "Audit Log",    icon: ScrollText },
  { to: "/admin/health",       label: "Health",       icon: HeartPulse },
  { to: "/admin/ip-blocks",    label: "IP Blocks",    icon: Shield },
  { to: "/admin/beta-channel",    label: "Beta Channel",    icon: FlaskConical },
  { to: "/admin/team-namespaces", label: "Team Namespaces", icon: FolderKey },
];

function isActive(to: string) {
  return route.path === to || route.path.startsWith(to + "/");
}
</script>

<template>
  <div class="flex gap-6 min-h-[calc(100vh-3.5rem-1px)]">
    <!-- Sidebar (desktop) -->
    <aside class="hidden md:flex flex-col w-52 shrink-0 border-r pr-4 pt-2">
      <div class="flex items-center gap-2 px-3 py-2 mb-2 text-xs font-semibold text-muted-foreground uppercase tracking-wide">
        <ShieldCheck class="h-3.5 w-3.5" />
        Admin
      </div>
      <nav class="flex flex-col gap-0.5">
        <RouterLink
          v-for="link in adminLinks"
          :key="link.to"
          :to="link.to"
          :class="[
            'flex items-center gap-2.5 px-3 py-2 rounded-md text-sm transition-colors',
            isActive(link.to)
              ? 'bg-accent text-accent-foreground font-medium'
              : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
          ]"
        >
          <component
            :is="link.icon"
            class="h-4 w-4 shrink-0"
          />
          {{ link.label }}
        </RouterLink>
      </nav>
    </aside>

    <!-- Mobile: horizontal tab strip -->
    <div class="md:hidden -mx-4 px-4 border-b mb-4 w-full flex flex-col">
      <div class="flex items-center gap-1 pb-1 overflow-x-auto">
        <span class="flex items-center gap-1 text-xs text-muted-foreground mr-2 shrink-0">
          <ShieldCheck class="h-3 w-3" /> Admin
        </span>
        <RouterLink
          v-for="link in adminLinks"
          :key="link.to"
          :to="link.to"
          :class="[
            'flex items-center gap-1.5 px-3 py-1.5 rounded-md text-sm whitespace-nowrap transition-colors shrink-0',
            isActive(link.to)
              ? 'bg-accent text-accent-foreground font-medium'
              : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
          ]"
        >
          <component
            :is="link.icon"
            class="h-3.5 w-3.5 shrink-0"
          />
          {{ link.label }}
        </RouterLink>
      </div>
    </div>

    <!-- Content -->
    <div class="flex-1 min-w-0 pt-2">
      <RouterView />
    </div>
  </div>
</template>
