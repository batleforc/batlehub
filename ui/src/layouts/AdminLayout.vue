<script setup lang="ts">
import { RouterView, RouterLink, useRoute } from "vue-router";
import {
  ShieldCheck,
  LayoutDashboard,
  Package,
  RefreshCw,
  HeartPulse,
  Shield,
  FolderKey,
  Bell,
} from "@lucide/vue";

const route = useRoute();

const adminLinks = [
  { to: "/admin/dashboard", label: "Dashboard", icon: LayoutDashboard },
  { to: "/admin/packages", label: "Packages", icon: Package },
  { to: "/admin/security", label: "Security & Access", icon: Shield },
  { to: "/admin/namespaces", label: "Namespaces & Channels", icon: FolderKey },
  { to: "/admin/operations", label: "Operations", icon: RefreshCw },
  { to: "/admin/observability", label: "Observability", icon: HeartPulse },
  { to: "/admin/notifications", label: "Notifications", icon: Bell },
];

function isActive(to: string) {
  return route.path === to || route.path.startsWith(to + "/");
}
</script>

<template>
  <div class="flex gap-6 min-h-[calc(100vh-3.5rem-1px)]">
    <!-- Sidebar (desktop) -->
    <aside class="hidden md:flex flex-col w-52 shrink-0 border-r border-border/60 pr-4 pt-2">
      <div
        class="flex items-center gap-2 px-3 py-2 mb-2 font-mono text-xs font-semibold uppercase tracking-wider text-copper"
      >
        <ShieldCheck class="h-3.5 w-3.5" />
        Admin
      </div>
      <nav class="flex flex-col gap-0.5">
        <RouterLink
          v-for="link in adminLinks"
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
          {{ link.label }}
        </RouterLink>
      </nav>
    </aside>

    <!-- Mobile: horizontal tab strip -->
    <div class="md:hidden -mx-4 px-4 border-b border-border/60 mb-4 w-full flex flex-col">
      <div class="flex items-center gap-1 pb-1 overflow-x-auto">
        <span class="flex items-center gap-1 font-mono text-xs text-copper mr-2 shrink-0">
          <ShieldCheck class="h-3 w-3" /> Admin
        </span>
        <RouterLink
          v-for="link in adminLinks"
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
