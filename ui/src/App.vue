<script setup lang="ts">
import { computed, ref } from "vue";
import { RouterView, RouterLink, useRoute, useRouter } from "vue-router";
import { Menu, X, Package, ShieldCheck, BookOpen } from "@lucide/vue";
import { useAuth } from "@/composables/useAuth";
import { DOCS_URL } from "@/config";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import ThemeToggle from "@/components/ThemeToggle.vue";

const { identity, isAdmin, isAuthenticated, logout } = useAuth();
const route = useRoute();
const router = useRouter();
const mobileOpen = ref(false);

// Any non-null auth_provider means the session came through OIDC or Kubernetes.
const isOidcUser = computed(
  () => isAuthenticated.value && !!identity.value?.auth_provider
);

const userLinks = [
  { to: "/packages", label: "Packages" },
  { to: "/access-check", label: "Access Check" },
  { to: "/path-mapper", label: "URL Mapper" },
  { to: "/setup", label: "Setup" },
];

const userInitials = computed(() => {
  const uid = identity.value?.user_id;
  if (!uid) return "?";
  return uid.slice(0, 2).toUpperCase();
});

function handleLogout() {
  logout();
  router.push("/login");
  mobileOpen.value = false;
}

function isActive(to: string) {
  return route.path === to || route.path.startsWith(to + "/");
}
</script>

<template>
  <div class="min-h-screen bg-background">
    <!-- Header -->
    <header class="sticky top-0 z-40 border-b bg-card/95 backdrop-blur supports-[backdrop-filter]:bg-card/60">
      <div class="container mx-auto flex h-14 items-center gap-4 px-4">
        <!-- Logo -->
        <RouterLink
          to="/packages"
          class="flex items-center gap-2 font-semibold text-sm hover:text-foreground/80 shrink-0"
        >
          <Package class="h-4 w-4 text-primary" />
          BatleHub
        </RouterLink>

        <!-- Desktop nav -->
        <nav class="hidden md:flex items-center gap-1 text-sm">
          <RouterLink
            v-for="link in userLinks"
            :key="link.to"
            :to="link.to"
            :class="[
              'px-3 py-1.5 rounded-md transition-colors hover:bg-accent hover:text-accent-foreground',
              isActive(link.to)
                ? 'bg-accent text-accent-foreground font-medium'
                : 'text-muted-foreground',
            ]"
          >
            {{ link.label }}
          </RouterLink>

          <!-- Tokens link for OIDC users -->
          <RouterLink
            v-if="isOidcUser"
            to="/tokens"
            :class="[
              'px-3 py-1.5 rounded-md transition-colors hover:bg-accent hover:text-accent-foreground',
              isActive('/tokens')
                ? 'bg-accent text-accent-foreground font-medium'
                : 'text-muted-foreground',
            ]"
          >
            My Tokens
          </RouterLink>

          <!-- My Profile link for authenticated users -->
          <RouterLink
            v-if="isAuthenticated"
            to="/profile"
            :class="[
              'px-3 py-1.5 rounded-md transition-colors hover:bg-accent hover:text-accent-foreground',
              isActive('/profile')
                ? 'bg-accent text-accent-foreground font-medium'
                : 'text-muted-foreground',
            ]"
          >
            My Profile
          </RouterLink>
        </nav>

        <!-- Admin entry point (desktop) -->
        <div v-if="isAdmin" class="hidden md:flex items-center gap-1">
          <div class="mx-2 h-4 w-px bg-border" />
          <RouterLink
            to="/admin/packages"
            :class="[
              'flex items-center gap-1.5 px-3 py-1.5 rounded-md transition-colors text-sm hover:bg-accent hover:text-accent-foreground',
              isActive('/admin')
                ? 'bg-accent text-accent-foreground font-medium'
                : 'text-muted-foreground',
            ]"
          >
            <ShieldCheck class="h-3.5 w-3.5" />
            Admin
          </RouterLink>
        </div>

        <!-- Right side -->
        <div class="ml-auto flex items-center gap-2">
          <a
            :href="DOCS_URL"
            target="_blank"
            rel="noopener noreferrer"
            class="hidden sm:flex items-center gap-1.5 px-3 py-1.5 rounded-md text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground"
          >
            <BookOpen class="h-3.5 w-3.5" />
            Docs
          </a>
          <ThemeToggle />

          <template v-if="isAuthenticated">
            <!-- Avatar + user info -->
            <div class="hidden sm:flex items-center gap-2">
              <div
                class="h-7 w-7 rounded-full bg-primary text-primary-foreground flex items-center justify-center text-xs font-semibold"
              >
                {{ userInitials }}
              </div>
              <span class="text-sm text-muted-foreground hidden lg:inline">
                {{ identity?.user_id }}
              </span>
              <Badge v-if="isAdmin" variant="secondary" class="text-xs">admin</Badge>
              <Badge v-else-if="identity?.role !== 'anonymous'" variant="outline" class="text-xs">
                {{ identity?.role }}
              </Badge>
            </div>
            <Button variant="ghost" size="sm" @click="handleLogout" class="text-sm">
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

          <!-- Mobile menu toggle -->
          <Button
            variant="ghost"
            size="icon"
            class="md:hidden"
            @click="mobileOpen = !mobileOpen"
          >
            <X v-if="mobileOpen" class="h-4 w-4" />
            <Menu v-else class="h-4 w-4" />
          </Button>
        </div>
      </div>

      <!-- Mobile nav -->
      <div v-if="mobileOpen" class="md:hidden border-t bg-card px-4 py-3 space-y-1">
        <RouterLink
          v-for="link in userLinks"
          :key="link.to"
          :to="link.to"
          :class="[
            'block px-3 py-2 rounded-md text-sm transition-colors',
            isActive(link.to)
              ? 'bg-accent text-accent-foreground font-medium'
              : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
          ]"
          @click="mobileOpen = false"
        >
          {{ link.label }}
        </RouterLink>
        <RouterLink
          v-if="isOidcUser"
          to="/tokens"
          :class="[
            'block px-3 py-2 rounded-md text-sm transition-colors',
            isActive('/tokens')
              ? 'bg-accent text-accent-foreground font-medium'
              : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
          ]"
          @click="mobileOpen = false"
        >
          My Tokens
        </RouterLink>
        <RouterLink
          v-if="isAuthenticated"
          to="/profile"
          :class="[
            'block px-3 py-2 rounded-md text-sm transition-colors',
            isActive('/profile')
              ? 'bg-accent text-accent-foreground font-medium'
              : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
          ]"
          @click="mobileOpen = false"
        >
          My Profile
        </RouterLink>
        <RouterLink
          v-if="isAdmin"
          to="/admin/packages"
          :class="[
            'flex items-center gap-2 px-3 py-2 rounded-md text-sm transition-colors',
            isActive('/admin')
              ? 'bg-accent text-accent-foreground font-medium'
              : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
          ]"
          @click="mobileOpen = false"
        >
          <ShieldCheck class="h-4 w-4" />
          Admin
        </RouterLink>
        <a
          :href="DOCS_URL"
          target="_blank"
          rel="noopener noreferrer"
          class="flex items-center gap-2 px-3 py-2 rounded-md text-sm text-muted-foreground hover:bg-accent hover:text-accent-foreground"
          @click="mobileOpen = false"
        >
          <BookOpen class="h-4 w-4" />
          Documentation
        </a>
        <div v-if="isAuthenticated" class="pt-2 border-t">
          <button
            class="block w-full text-left px-3 py-2 rounded-md text-sm text-muted-foreground hover:bg-accent hover:text-accent-foreground"
            @click="handleLogout"
          >
            Sign out
          </button>
        </div>
      </div>
    </header>

    <main class="container mx-auto px-4 py-6">
      <RouterView />
    </main>
  </div>
</template>
