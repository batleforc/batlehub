<script setup lang="ts">
import { computed } from "vue";
import { Users, User, Shield, KeyRound } from "@lucide/vue";
import { useAuth } from "@/composables/useAuth";
import { Badge } from "@/components/ui/badge";
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@/components/ui/card";

const { identity, oidcProvider } = useAuth();

// Prefer the SSO provider stored at login time (reflects which button the user clicked)
// over identity.auth_provider (which reflects which JWT validator processed the token —
// these differ when multiple providers share the same issuer URL).
const displayProvider = computed(() => oidcProvider.value || identity.value?.auth_provider || null);

// Parse "provider:groupname" into parts for visual display.
// Groups without a colon are stored as plain values (e.g. mapped role names).
const parsedGroups = computed(() => {
  if (!identity.value?.groups) return [];
  return identity.value.groups.map((g) => {
    const colon = g.indexOf(":");
    if (colon === -1) return { raw: g, provider: null, name: g };
    return { raw: g, provider: g.slice(0, colon), name: g.slice(colon + 1) };
  });
});

function roleVariant(role: string) {
  if (role === "admin") return "destructive" as const;
  if (role === "user") return "secondary" as const;
  return "outline" as const;
}
</script>

<template>
  <div class="space-y-6 max-w-2xl">
    <!-- Identity card -->
    <Card>
      <CardHeader>
        <CardTitle class="flex items-center gap-2 text-lg">
          <User class="h-4 w-4" />
          Identity
        </CardTitle>
        <CardDescription>
          Your current session information.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <dl class="grid grid-cols-[auto_1fr] gap-x-6 gap-y-3 text-sm">
          <dt class="text-muted-foreground font-medium">
            User ID
          </dt>
          <dd class="font-mono">
            {{ identity?.user_id ?? "—" }}
          </dd>

          <dt class="text-muted-foreground font-medium">
            Role
          </dt>
          <dd>
            <Badge :variant="roleVariant(identity?.role ?? 'anonymous')">
              {{ identity?.role ?? "anonymous" }}
            </Badge>
          </dd>

          <dt class="text-muted-foreground font-medium">
            Auth provider
          </dt>
          <dd>
            <span
              v-if="displayProvider"
              class="flex items-center gap-1.5"
            >
              <KeyRound class="h-3.5 w-3.5 text-muted-foreground" />
              <span class="font-mono">{{ displayProvider }}</span>
            </span>
            <span
              v-else
              class="text-muted-foreground"
            >Token / anonymous</span>
          </dd>
        </dl>
      </CardContent>
    </Card>

    <!-- Groups card -->
    <Card>
      <CardHeader>
        <CardTitle class="flex items-center gap-2 text-lg">
          <Users class="h-4 w-4" />
          Groups
          <span
            v-if="parsedGroups.length"
            class="ml-1 text-muted-foreground font-normal text-base"
          >
            ({{ parsedGroups.length }})
          </span>
        </CardTitle>
        <CardDescription>
          Dynamic groups assigned by your identity provider.
          Groups with a provider prefix (e.g. <code class="font-mono text-xs">oidc:team-a</code>)
          are scoped to that provider; unprefixed values were mapped directly to a role.
        </CardDescription>
      </CardHeader>

      <CardContent>
        <!-- No groups -->
        <div
          v-if="!parsedGroups.length"
          class="py-8 text-center space-y-2"
        >
          <Shield class="h-8 w-8 mx-auto text-muted-foreground/50" />
          <p class="text-sm text-muted-foreground">
            No groups assigned to this session.
          </p>
          <p class="text-xs text-muted-foreground">
            Groups are populated when you authenticate via an OIDC or Kubernetes provider
            that includes group claims.
          </p>
        </div>

        <!-- Group list -->
        <ul
          v-else
          class="space-y-2"
        >
          <li
            v-for="g in parsedGroups"
            :key="g.raw"
            class="flex items-center gap-1.5 font-mono text-sm"
          >
            <!-- Provider prefix as a muted badge -->
            <template v-if="g.provider">
              <Badge
                variant="outline"
                class="text-xs text-muted-foreground"
              >
                {{ g.provider }}
              </Badge>
              <span class="text-muted-foreground select-none">:</span>
            </template>
            <!-- Group name -->
            <span>{{ g.name }}</span>
          </li>
        </ul>
      </CardContent>
    </Card>
  </div>
</template>
