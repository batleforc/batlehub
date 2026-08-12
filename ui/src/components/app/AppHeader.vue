<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { computed } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";
import { Menu, X, Package, ShieldCheck, BookOpen, FolderKey } from "@lucide/vue";
import { useAuth } from "@/composables/useAuth";
import { primaryNav } from "@/config/navigation";
import { DOCS_URL } from "@/config";
import { Button } from "@/components/ui/button";
import ThemeToggle from "@/components/ThemeToggle.vue";
import LocaleToggle from "@/components/LocaleToggle.vue";
import AppNav from "./AppNav.vue";
import UserMenu from "./UserMenu.vue";
import GlobalBanner from "./GlobalBanner.vue";

const { t } = useI18n();

const mobileOpen = defineModel<boolean>("mobileOpen", { default: false });

const { identity, isAdmin, isAuthenticated, logout } = useAuth();
const route = useRoute();
const router = useRouter();

const isOidcUser = computed(() => isAuthenticated.value && !!identity.value?.auth_provider);

/* The bar follows the viewer (RFC 0003 §4.1): an item appears when it can be
   used, so nothing here leads to a redirect. Diagnostics moved to /tools —
   they are linked from the errors that motivate them instead of holding
   primary-nav weight for the rest of the year. */
const navLinks = computed(() =>
  primaryNav({
    isAuthenticated: isAuthenticated.value,
    isAdmin: isAdmin.value,
    hasRegistryAccess: identity.value?.has_registry_access !== false,
  }),
);
const userLinks = computed(() => navLinks.value.filter((l) => l.to !== "/admin"));
const adminLink = computed(() => navLinks.value.find((l) => l.to === "/admin"));

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
  <header class="sticky top-0 z-40 border-b border-border/60 bg-background/90 backdrop-blur-md">
    <div class="container mx-auto flex h-14 items-center gap-4 px-4">
      <!-- The wordmark goes home. It pointed at /packages from when `/` was a
           blind redirect there; Phase 4 made `/` a real identity-aware surface
           (RFC 0003 §4.3), so the logo had been skipping past it ever since. -->
      <RouterLink
        to="/"
        class="flex items-center gap-2 shrink-0 font-display font-bold text-base tracking-[0.04em] text-primary hover:text-primary/80 transition-colors"
      >
        <Package class="h-4 w-4" />
        BatleHub.
      </RouterLink>

      <AppNav :links="userLinks" variant="desktop" />

      <!-- Admin entry point (desktop) -->
      <div v-if="adminLink" class="hidden md:flex items-center gap-1">
        <div class="mx-2 h-4 w-px bg-border" />
        <RouterLink
          to="/admin"
          :class="[
            'flex items-center gap-1.5 px-3 py-1.5 rounded-sm font-mono text-sm transition-colors',
            isActive('/admin')
              ? 'bg-accent text-accent-foreground font-semibold'
              : 'text-copper hover:bg-accent/60 hover:text-accent-foreground',
          ]"
        >
          <ShieldCheck class="h-3.5 w-3.5" />
          {{ t("common.admin") }}
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
          {{ t("common.docs") }}
        </a>
        <LocaleToggle />
        <ThemeToggle />

        <UserMenu />

        <!-- Mobile menu toggle -->
        <Button
          variant="ghost"
          size="icon"
          class="md:hidden"
          :aria-label="mobileOpen ? t('a11y.closeMenu') : t('a11y.openMenu')"
          :aria-expanded="mobileOpen"
          aria-controls="mobile-nav"
          @click="mobileOpen = !mobileOpen"
        >
          <X v-if="mobileOpen" class="h-4 w-4" />
          <Menu v-else class="h-4 w-4" />
        </Button>
      </div>
    </div>

    <!-- Mobile nav -->
    <div
      v-if="mobileOpen"
      id="mobile-nav"
      class="md:hidden border-t border-border/60 bg-card px-4 py-3 space-y-1"
    >
      <AppNav :links="userLinks" variant="mobile" @navigate="mobileOpen = false" />
      <RouterLink
        v-if="isOidcUser"
        to="/me/tokens"
        :class="[
          'block px-3 py-2 rounded-sm font-mono text-sm transition-colors',
          isActive('/me/tokens')
            ? 'bg-accent text-accent-foreground font-semibold'
            : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
        ]"
        @click="mobileOpen = false"
        >{{ t("appHeader.myTokens") }}</RouterLink
      >
      <RouterLink
        v-if="isAuthenticated"
        to="/me/profile"
        :class="[
          'block px-3 py-2 rounded-sm font-mono text-sm transition-colors',
          isActive('/me/profile')
            ? 'bg-accent text-accent-foreground font-semibold'
            : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
        ]"
        @click="mobileOpen = false"
        >{{ t("appHeader.myProfile") }}</RouterLink
      >
      <RouterLink
        v-if="isAuthenticated"
        to="/me/namespace"
        :class="[
          'flex items-center gap-2 px-3 py-2 rounded-sm font-mono text-sm transition-colors',
          isActive('/me/namespace')
            ? 'bg-accent text-accent-foreground font-semibold'
            : 'text-muted-foreground hover:bg-accent hover:text-accent-foreground',
        ]"
        @click="mobileOpen = false"
      >
        <FolderKey class="h-4 w-4" />
        {{ t("appHeader.myNamespace") }}
      </RouterLink>
      <RouterLink
        v-if="isAdmin"
        to="/admin"
        :class="[
          'flex items-center gap-2 px-3 py-2 rounded-sm font-mono text-sm transition-colors',
          isActive('/admin')
            ? 'bg-accent text-accent-foreground font-semibold'
            : 'text-copper hover:bg-accent hover:text-accent-foreground',
        ]"
        @click="mobileOpen = false"
      >
        <ShieldCheck class="h-4 w-4" />
        {{ t("common.admin") }}
      </RouterLink>
      <a
        :href="DOCS_URL"
        target="_blank"
        rel="noopener noreferrer"
        class="flex items-center gap-2 px-3 py-2 rounded-sm font-mono text-sm text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors"
        @click="mobileOpen = false"
      >
        <BookOpen class="h-4 w-4" />
        {{ t("common.documentation") }}
      </a>
      <div v-if="isAuthenticated" class="pt-2 border-t border-border/60">
        <button
          class="block w-full text-left px-3 py-2 rounded-sm font-mono text-sm text-destructive hover:bg-destructive/10 transition-colors"
          @click="handleLogout"
        >
          {{ t("appHeader.signOut") }}
        </button>
      </div>
    </div>

    <GlobalBanner />
  </header>
</template>
