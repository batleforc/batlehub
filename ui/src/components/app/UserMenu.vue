<script setup lang="ts">
import { computed } from "vue";
import { RouterLink, useRouter } from "vue-router";
import { User, KeyRound, FolderKey, Terminal, LogOut, ChevronDown } from "@lucide/vue";
import {
  DropdownMenuRoot,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuLabel,
} from "radix-vue";
import { useAuth } from "@/composables/useAuth";
import { Badge } from "@/components/ui/badge";

const { identity, isAdmin, isAuthenticated, logout } = useAuth();
const router = useRouter();

const isOidcUser = computed(() => isAuthenticated.value && !!identity.value?.auth_provider);

const userInitials = computed(() => {
  const uid = identity.value?.user_id;
  if (!uid) return "?";
  return uid.slice(0, 2).toUpperCase();
});

function handleLogout() {
  logout();
  router.push("/login");
}
</script>

<template>
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
</template>
