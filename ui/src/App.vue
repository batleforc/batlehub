<script setup lang="ts">
import { computed } from "vue";
import { RouterView, RouterLink, useRoute, useRouter } from "vue-router";
import { useAuth } from "@/composables/useAuth";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

const { identity, isAdmin, isAuthenticated, logout } = useAuth();
const route = useRoute();
const router = useRouter();

const navLinks = computed(() => {
  const links: { to: string; label: string }[] = [
    { to: "/packages", label: "Packages" },
    { to: "/access-check", label: "Access Check" },
    { to: "/path-mapper", label: "URL Mapper" },
    { to: "/setup", label: "Setup" },
  ];
  if (isAdmin.value) {
    links.push(
      { to: "/admin/packages", label: "Admin — Packages" },
      { to: "/admin/audit-log", label: "Admin — Audit Log" },
    );
  }
  return links;
});

function handleLogout() {
  logout();
  router.push("/login");
}
</script>

<template>
  <div class="min-h-screen bg-background">
    <header class="border-b bg-card">
      <div class="container mx-auto flex h-14 items-center gap-6 px-4">
        <RouterLink to="/packages" class="font-semibold text-sm hover:text-foreground/80">
          Proxy Cache
        </RouterLink>

        <nav class="flex items-center gap-4 text-sm">
          <RouterLink
            v-for="link in navLinks"
            :key="link.to"
            :to="link.to"
            :class="[
              'transition-colors hover:text-foreground/80',
              route.path === link.to
                ? 'text-foreground font-medium'
                : 'text-muted-foreground',
            ]"
          >
            {{ link.label }}
          </RouterLink>
        </nav>

        <div class="ml-auto flex items-center gap-3">
          <template v-if="isAuthenticated">
            <span class="text-sm font-medium">
              {{ identity?.user_id }}
            </span>
            <Badge v-if="!isAdmin" variant="secondary">{{ identity?.role }}</Badge>
            <Button variant="ghost" size="sm" @click="handleLogout">
              Sign out
            </Button>
          </template>
          <RouterLink
            v-else
            to="/login"
            class="text-sm text-muted-foreground hover:text-foreground transition-colors"
          >
            Sign in
          </RouterLink>
        </div>
      </div>
    </header>

    <main class="container mx-auto px-4 py-6">
      <RouterView />
    </main>
  </div>
</template>
