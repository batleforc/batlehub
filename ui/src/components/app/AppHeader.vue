<script setup lang="ts">
import { computed } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";
import { Menu, X, Package, ShieldCheck, BookOpen, FolderKey } from "@lucide/vue";
import { useAuth } from "@/composables/useAuth";
import { DOCS_URL } from "@/config";
import { Button } from "@/components/ui/button";
import ThemeToggle from "@/components/ThemeToggle.vue";
import AppNav from "./AppNav.vue";
import UserMenu from "./UserMenu.vue";
import GlobalBanner from "./GlobalBanner.vue";

const mobileOpen = defineModel<boolean>("mobileOpen", { default: false });

const { identity, isAdmin, isAuthenticated, logout } = useAuth();
const route = useRoute();
const router = useRouter();

const isOidcUser = computed(() => isAuthenticated.value && !!identity.value?.auth_provider);

const userLinks = [
  { to: "/packages", label: "Packages" },
  { to: "/explore", label: "Explore" },
  { to: "/access-check", label: "Access Check" },
  { to: "/path-mapper", label: "URL Mapper" },
  { to: "/setup", label: "Setup" },
];

function isActive(to: string) {
  return route.path === to || route.path.startsWith(to + "/");
}

function handleLogout() {
  logout();
  router.push("/login");
  mobileOpen.value = false;
}
</script>

<template>
  <header
    class="sticky top-0 z-40 cyber-grid-bg border-b border-border/60 bg-background/90 backdrop-blur-md"
  >
    <div class="container mx-auto flex h-14 items-center gap-4 px-4">
      <!-- Logo -->
      <RouterLink
        to="/packages"
        class="flex items-center gap-2 shrink-0 font-mono font-bold text-base text-primary cyber-text-glow hover:text-primary/80 transition-colors"
      >
        <Package class="h-4 w-4" />
        BatleHub.
      </RouterLink>

      <AppNav :links="userLinks" variant="desktop" />

      <!-- Admin entry point (desktop) -->
      <div v-if="isAdmin" class="hidden md:flex items-center gap-1">
        <div class="mx-2 h-4 w-px bg-border" />
        <RouterLink
          to="/admin/packages"
          :class="[
            'flex items-center gap-1.5 px-3 py-1.5 rounded-sm font-mono text-sm transition-colors',
            isActive('/admin')
              ? 'bg-accent text-accent-foreground font-semibold'
              : 'text-copper hover:bg-accent/60 hover:text-accent-foreground',
          ]"
        >
          <ShieldCheck class="h-3.5 w-3.5" />
          Admin
        </RouterLink>
      </div>

      <!-- Right side -->
      <div class="ml-auto flex items-center gap-1.5">
        <a
          :href="DOCS_URL"
          target="_blank"
          rel="noopener noreferrer"
          class="hidden sm:flex items-center gap-1.5 px-3 py-1.5 rounded-sm font-mono text-sm text-muted-foreground transition-colors hover:bg-accent/60 hover:text-accent-foreground"
        >
          <BookOpen class="h-3.5 w-3.5" />
          Docs
        </a>
        <ThemeToggle />

        <UserMenu />

        <!-- Mobile menu toggle -->
        <Button variant="ghost" size="icon" class="md:hidden" @click="mobileOpen = !mobileOpen">
          <X v-if="mobileOpen" class="h-4 w-4" />
          <Menu v-else class="h-4 w-4" />
        </Button>
      </div>
    </div>

    <!-- Mobile nav -->
    <div v-if="mobileOpen" class="md:hidden border-t border-border/60 bg-card px-4 py-3 space-y-1">
      <AppNav :links="userLinks" variant="mobile" @navigate="mobileOpen = false" />
      <RouterLink
        v-if="isOidcUser"
        to="/tokens"
        :class="[
          'block px-3 py-2 rounded-sm font-mono text-sm transition-colors',
          isActive('/tokens')
            ? 'bg-accent text-accent-foreground font-semibold'
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
          'block px-3 py-2 rounded-sm font-mono text-sm transition-colors',
          isActive('/profile')
            ? 'bg-accent text-accent-foreground font-semibold'
            : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
        ]"
        @click="mobileOpen = false"
      >
        My Profile
      </RouterLink>
      <RouterLink
        v-if="isAuthenticated"
        to="/my-namespace"
        :class="[
          'flex items-center gap-2 px-3 py-2 rounded-sm font-mono text-sm transition-colors',
          isActive('/my-namespace')
            ? 'bg-accent text-accent-foreground font-semibold'
            : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
        ]"
        @click="mobileOpen = false"
      >
        <FolderKey class="h-4 w-4" />
        My Namespace
      </RouterLink>
      <RouterLink
        v-if="isAdmin"
        to="/admin/packages"
        :class="[
          'flex items-center gap-2 px-3 py-2 rounded-sm font-mono text-sm transition-colors',
          isActive('/admin')
            ? 'bg-accent text-accent-foreground font-semibold'
            : 'text-copper hover:bg-accent hover:text-accent-foreground',
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
        class="flex items-center gap-2 px-3 py-2 rounded-sm font-mono text-sm text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors"
        @click="mobileOpen = false"
      >
        <BookOpen class="h-4 w-4" />
        Documentation
      </a>
      <div v-if="isAuthenticated" class="pt-2 border-t border-border/60">
        <button
          class="block w-full text-left px-3 py-2 rounded-sm font-mono text-sm text-destructive hover:bg-destructive/10 transition-colors"
          @click="handleLogout"
        >
          Sign out
        </button>
      </div>
    </div>

    <GlobalBanner />
  </header>
</template>
