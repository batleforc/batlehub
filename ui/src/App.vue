<script setup lang="ts">
import { computed, ref } from "vue";
import { RouterView, RouterLink, useRoute, useRouter } from "vue-router";
import {
  AlertTriangle,
  Info,
  Menu,
  X,
  XCircle,
  Package,
  ShieldCheck,
  BookOpen,
  FolderKey,
  User,
  KeyRound,
  LogOut,
  ChevronDown,
  Terminal,
} from "@lucide/vue";
import {
  DropdownMenuRoot,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuLabel,
} from "radix-vue";
import { useAuth } from "@/composables/useAuth";
import { useBanner } from "@/composables/useBanner";
import { DOCS_URL } from "@/config";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import ThemeToggle from "@/components/ThemeToggle.vue";

const { identity, isAdmin, isAuthenticated, logout } = useAuth();
const { banner } = useBanner();
const route = useRoute();
const router = useRouter();
const mobileOpen = ref(false);

const isOidcUser = computed(() => isAuthenticated.value && !!identity.value?.auth_provider);

const userLinks = [
  { to: "/packages", label: "Packages" },
  { to: "/explore", label: "Explore" },
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
    <!-- Sticky header with circuit-grid background -->
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

        <!-- Desktop nav -->
        <nav class="hidden md:flex items-center gap-0.5 text-sm">
          <RouterLink
            v-for="link in userLinks"
            :key="link.to"
            :to="link.to"
            :class="[
              'px-3 py-1.5 rounded-sm font-mono text-sm transition-colors',
              isActive(link.to)
                ? 'bg-accent text-accent-foreground font-semibold'
                : 'text-muted-foreground hover:bg-accent/60 hover:text-accent-foreground',
            ]"
          >
            {{ link.label }}
          </RouterLink>
        </nav>

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

          <template v-if="isAuthenticated">
            <DropdownMenuRoot>
              <DropdownMenuTrigger
                class="hidden sm:flex items-center gap-1.5 rounded-sm px-1.5 py-1 text-sm hover:bg-accent/60 transition-colors outline-none"
              >
                <div
                  class="h-7 w-7 rounded-sm bg-primary text-primary-foreground flex items-center justify-center text-xs font-mono font-bold shrink-0"
                >
                  {{ userInitials }}
                </div>
                <span
                  class="text-muted-foreground hidden lg:inline max-w-[10rem] truncate font-mono text-xs"
                >
                  {{ identity?.user_id }}
                </span>
                <Badge v-if="isAdmin" variant="copper" class="text-xs">admin</Badge>
                <Badge v-else-if="identity?.role !== 'anonymous'" variant="outline" class="text-xs">
                  {{ identity?.role }}
                </Badge>
                <ChevronDown class="h-3.5 w-3.5 text-muted-foreground shrink-0" />
              </DropdownMenuTrigger>
              <DropdownMenuContent
                align="end"
                class="z-50 min-w-[11rem] rounded-sm border border-border bg-popover p-1 shadow-[var(--cyber-glow)]"
              >
                <DropdownMenuLabel
                  class="px-2 py-1.5 text-xs text-muted-foreground font-mono font-normal truncate"
                >
                  {{ identity?.user_id }}
                </DropdownMenuLabel>
                <DropdownMenuSeparator class="my-1 h-px bg-border" />
                <DropdownMenuItem as-child>
                  <RouterLink
                    to="/profile"
                    class="flex items-center gap-2 px-2 py-1.5 text-sm rounded-sm cursor-pointer hover:bg-accent hover:text-accent-foreground outline-none transition-colors"
                  >
                    <User class="h-3.5 w-3.5" />
                    My Profile
                  </RouterLink>
                </DropdownMenuItem>
                <DropdownMenuItem v-if="isOidcUser" as-child>
                  <RouterLink
                    to="/tokens"
                    class="flex items-center gap-2 px-2 py-1.5 text-sm rounded-sm cursor-pointer hover:bg-accent hover:text-accent-foreground outline-none transition-colors"
                  >
                    <KeyRound class="h-3.5 w-3.5" />
                    My Tokens
                  </RouterLink>
                </DropdownMenuItem>
                <DropdownMenuItem as-child>
                  <RouterLink
                    to="/my-namespace"
                    class="flex items-center gap-2 px-2 py-1.5 text-sm rounded-sm cursor-pointer hover:bg-accent hover:text-accent-foreground outline-none transition-colors"
                  >
                    <FolderKey class="h-3.5 w-3.5" />
                    My Namespace
                  </RouterLink>
                </DropdownMenuItem>
                <DropdownMenuItem as-child>
                  <RouterLink
                    to="/cli"
                    class="flex items-center gap-2 px-2 py-1.5 text-sm rounded-sm cursor-pointer hover:bg-accent hover:text-accent-foreground outline-none transition-colors"
                  >
                    <Terminal class="h-3.5 w-3.5" />
                    Download CLI
                  </RouterLink>
                </DropdownMenuItem>
                <DropdownMenuSeparator class="my-1 h-px bg-border" />
                <DropdownMenuItem
                  class="flex items-center gap-2 px-2 py-1.5 text-sm rounded-sm cursor-pointer hover:bg-destructive/10 text-destructive hover:text-destructive outline-none transition-colors"
                  @select="handleLogout"
                >
                  <LogOut class="h-3.5 w-3.5" />
                  Sign out
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenuRoot>
          </template>
          <RouterLink
            v-else
            to="/login"
            class="font-mono text-sm text-muted-foreground hover:text-primary transition-colors"
          >
            Sign in
          </RouterLink>

          <!-- Mobile menu toggle -->
          <Button variant="ghost" size="icon" class="md:hidden" @click="mobileOpen = !mobileOpen">
            <X v-if="mobileOpen" class="h-4 w-4" />
            <Menu v-else class="h-4 w-4" />
          </Button>
        </div>
      </div>

      <!-- Mobile nav -->
      <div
        v-if="mobileOpen"
        class="md:hidden border-t border-border/60 bg-card px-4 py-3 space-y-1"
      >
        <RouterLink
          v-for="link in userLinks"
          :key="link.to"
          :to="link.to"
          :class="[
            'block px-3 py-2 rounded-sm font-mono text-sm transition-colors',
            isActive(link.to)
              ? 'bg-accent text-accent-foreground font-semibold'
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

      <!-- Global admin banner -->
      <div
        v-if="banner"
        :class="[
          'border-t px-4 py-1.5 flex items-center gap-2 text-sm font-mono font-medium',
          banner.level === 'error'
            ? 'bg-destructive/10 border-destructive/40 text-destructive'
            : banner.level === 'warning'
              ? 'bg-copper/10 border-copper/40 text-copper'
              : 'bg-primary/10 border-primary/30 text-primary',
        ]"
      >
        <XCircle v-if="banner.level === 'error'" class="h-3.5 w-3.5 shrink-0" />
        <AlertTriangle v-else-if="banner.level === 'warning'" class="h-3.5 w-3.5 shrink-0" />
        <Info v-else class="h-3.5 w-3.5 shrink-0" />
        <span>{{ banner.message }}</span>
      </div>
    </header>

    <main class="container mx-auto px-4 py-6">
      <RouterView />
    </main>
  </div>
</template>
