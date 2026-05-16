<script setup lang="ts">
import { RouterView, RouterLink, useRoute } from "vue-router";
import { useAuth } from "@/composables/useAuth";
import { useApi } from "@/composables/useApi";
import { me } from "@/client/sdk.gen";
import type { MeResponse } from "@/client/types.gen";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { watch } from "vue";

const { token } = useAuth();
const route = useRoute();

const { data: identity, reload: reloadIdentity } = useApi<MeResponse>(
  () => me() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

watch(token, () => reloadIdentity());

const navLinks = [
  { to: "/packages", label: "Packages" },
  { to: "/access-check", label: "Access Check" },
  { to: "/admin/packages", label: "Admin — Packages" },
  { to: "/admin/audit-log", label: "Admin — Audit Log" },
];
</script>

<template>
  <div class="min-h-screen bg-background">
    <header class="border-b bg-card">
      <div class="container mx-auto flex h-14 items-center gap-6 px-4">
        <span class="font-semibold text-sm">Proxy Cache</span>

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
          <Input
            v-model="token"
            type="password"
            placeholder="Bearer token (optional)"
            class="h-8 w-64 text-xs"
          />
          <div v-if="identity" class="flex items-center gap-2 text-xs text-muted-foreground">
            <span>{{ identity.user_id ?? "anonymous" }}</span>
            <Badge variant="secondary">{{ identity.role }}</Badge>
          </div>
        </div>
      </div>
    </header>

    <main class="container mx-auto px-4 py-6">
      <RouterView />
    </main>
  </div>
</template>
