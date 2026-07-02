<script setup lang="ts">
import { ref, computed } from "vue";
import { RouterLink } from "vue-router";
import { API_BASE_URL } from "@/config";
import { listRegistries } from "@/client/sdk.gen";
import type { RegistryInfo } from "@/client/types.gen";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { PageHeader } from "@/components/ui/page-header";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import Select from "@/components/ui/select/Select.vue";
import CodeBlock from "@/components/ui/code-block/CodeBlock.vue";
import { Card, CardHeader, CardDescription, CardContent } from "@/components/ui/card";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import {
  REGISTRY_TYPE_DEFS,
  type RegistryTypeDef,
  type SnippetContext,
} from "@/config/registryTypes";

const base = computed(() => API_BASE_URL || globalThis.location.origin);
const copied = ref<string | null>(null);

const { token, identity, isAuthenticated, expiresAt } = useAuth();

const netrcHost = computed(() => {
  try {
    return new URL(base.value).hostname;
  } catch {
    return base.value;
  }
});
const netrcLogin = computed(() => identity.value?.user_id ?? "token");
const netrcSnippet = computed(
  () => `machine ${netrcHost.value}\nlogin ${netrcLogin.value}\npassword ${token.value}`,
);
const isOidc = computed(() => expiresAt.value > 0);

const { data: registries, loading } = useApi<Array<RegistryInfo>>(
  () => listRegistries() as Promise<{ data?: unknown; error?: unknown }>,
  [],
);

// Group API registries by type
const registriesByType = computed<Record<string, RegistryInfo[]>>(() => {
  const map: Record<string, RegistryInfo[]> = {};
  for (const r of registries.value ?? []) {
    map[r.type] ??= [];
    map[r.type].push(r);
  }
  return map;
});

// Per-type selected registry name; defaults to first in the list
const selectedByType = ref<Record<string, string>>({});

function getSelected(typeId: string): string {
  const list = registriesByType.value[typeId] ?? [];
  return selectedByType.value[typeId] ?? list[0]?.name ?? "";
}

function getMode(typeId: string): string {
  const name = getSelected(typeId);
  return registriesByType.value[typeId]?.find((r) => r.name === name)?.mode ?? "proxy";
}

// Map of all type → selected name, used by composite tabs (mise)
const selectedNames = computed<Record<string, string>>(() => {
  const result: Record<string, string> = {};
  for (const typeId of Object.keys(registriesByType.value)) {
    result[typeId] = getSelected(typeId);
  }
  return result;
});

// Show only tabs for registry types that are actually configured
const activeDefs = computed(() =>
  REGISTRY_TYPE_DEFS.filter((def) => {
    const types = def.apiTypes ?? [def.id];
    return types.some((t) => t in registriesByType.value);
  }),
);

const defaultTab = computed(
  () => activeDefs.value[0]?.id ?? (isAuthenticated.value ? "netrc" : ""),
);

// The primary API type for a tab (null for composite tabs with multiple types)
function primaryType(def: RegistryTypeDef): string | null {
  if (def.apiTypes && def.apiTypes.length > 1) return null;
  return def.apiTypes?.[0] ?? def.id;
}

function ctxFor(def: RegistryTypeDef): SnippetContext {
  const pt =
    primaryType(def) ??
    (def.apiTypes ?? [def.id]).find((t) => t in registriesByType.value) ??
    def.id;
  return {
    base: base.value,
    registryName: getSelected(pt),
    mode: getMode(pt),
    isAuthenticated: isAuthenticated.value,
    token: token.value ?? "",
    netrcHost: netrcHost.value,
    netrcLogin: netrcLogin.value,
    identity: identity.value,
    selectedNames: selectedNames.value,
  };
}

function selectorOptions(def: RegistryTypeDef) {
  const pt = primaryType(def);
  if (!pt) return [];
  return (registriesByType.value[pt] ?? []).map((r) => ({
    value: r.name,
    label: r.name,
  }));
}

function showSelector(def: RegistryTypeDef): boolean {
  return selectorOptions(def).length > 1;
}

async function copy(key: string, text: string) {
  await navigator.clipboard.writeText(text);
  copied.value = key;
  setTimeout(() => {
    copied.value = null;
  }, 1500);
}
</script>

<template>
  <div class="max-w-7xl space-y-8">
    <PageHeader
      title="Setup Guide"
      description="Configure your tools to route package downloads through this proxy. Snippets are pre-filled with this server's address and your configured registries."
      variant="glow"
    />

    <!-- Loading state -->
    <div v-if="loading" class="text-sm text-muted-foreground">Loading registries…</div>

    <!-- No registries configured -->
    <div
      v-else-if="activeDefs.length === 0 && !isAuthenticated"
      class="text-sm text-muted-foreground"
    >
      No registries are configured yet, or you don't have access to any. Contact your administrator
      or check your
      <code class="font-mono bg-muted px-1 rounded">config.toml</code>.
    </div>

    <!-- Tabs -->
    <Tabs v-else :default-value="defaultTab">
      <TabsList
        class="flex flex-wrap h-auto gap-1 justify-start bg-transparent border-none p-0 mb-2"
      >
        <TabsTrigger v-for="def in activeDefs" :key="def.id" :value="def.id" class="rounded-sm">
          {{ def.label }}
        </TabsTrigger>
        <TabsTrigger v-if="isAuthenticated" value="netrc" class="rounded-sm"> .netrc </TabsTrigger>
      </TabsList>

      <!-- Dynamic registry tabs -->
      <TabsContent v-for="def in activeDefs" :key="def.id" :value="def.id">
        <Card>
          <CardHeader>
            <div class="flex items-start justify-between gap-4">
              <div class="flex-1 space-y-3">
                <!-- Description (trusted HTML) -->
                <CardDescription>
                  <span v-html="def.description" />
                </CardDescription>
                <!-- Registry selector (shown when multiple registries of same type) -->
                <div v-if="showSelector(def)" class="flex items-center gap-2">
                  <label
                    :for="`setup-registry-${def.id}`"
                    class="text-xs text-muted-foreground shrink-0"
                    >Registry:</label
                  >
                  <Select
                    :id="`setup-registry-${def.id}`"
                    :model-value="getSelected(primaryType(def)!)"
                    :options="selectorOptions(def)"
                    class="h-7 text-xs w-48"
                    @update:model-value="selectedByType[primaryType(def)!] = $event"
                  />
                </div>
              </div>
              <Badge
                v-if="def.fileHint"
                variant="outline"
                class="shrink-0 font-mono text-xs mt-0.5"
              >
                {{ def.fileHint }}
              </Badge>
            </div>
          </CardHeader>

          <CardContent class="space-y-4">
            <template v-for="snippet in def.snippets" :key="snippet.key">
              <div v-if="!snippet.showWhen || snippet.showWhen(ctxFor(def))">
                <p v-if="snippet.label" class="text-xs text-muted-foreground mb-1.5">
                  {{ snippet.label }}
                </p>
                <CodeBlock :code="snippet.template(ctxFor(def))" :lang="snippet.lang">
                  <Button
                    size="sm"
                    variant="ghost"
                    class="absolute top-2 right-2 h-7 px-2 text-xs"
                    @click="copy(snippet.key, snippet.template(ctxFor(def)))"
                  >
                    {{ copied === snippet.key ? "Copied!" : "Copy" }}
                  </Button>
                </CodeBlock>
                <p
                  v-if="snippet.note"
                  class="text-xs text-muted-foreground mt-1.5"
                  v-html="
                    typeof snippet.note === 'function' ? snippet.note(ctxFor(def)) : snippet.note
                  "
                />
              </div>
            </template>
          </CardContent>
        </Card>
      </TabsContent>

      <!-- .netrc tab (authenticated users only) -->
      <TabsContent v-if="isAuthenticated" value="netrc">
        <Card>
          <CardHeader>
            <div class="flex items-center justify-between">
              <CardDescription>
                Credentials for tools that use HTTP Basic Auth (curl, wget, …). Place in
                <code class="text-xs font-mono bg-muted px-1 rounded">~/.netrc</code>
                and restrict permissions with
                <code class="text-xs font-mono bg-muted px-1 rounded">chmod 600 ~/.netrc</code>.
              </CardDescription>
              <Badge variant="outline" class="shrink-0 font-mono text-xs ml-4"> ~/.netrc </Badge>
            </div>
          </CardHeader>
          <CardContent class="space-y-3">
            <CodeBlock :code="netrcSnippet" lang="ini">
              <Button
                size="sm"
                variant="ghost"
                class="absolute top-2 right-2 h-7 px-2 text-xs"
                @click="copy('netrc', netrcSnippet)"
              >
                {{ copied === "netrc" ? "Copied!" : "Copy" }}
              </Button>
            </CodeBlock>
            <p v-if="isOidc" class="text-xs text-muted-foreground">
              Your current token is a short-lived OIDC session token. For long-lived automation,
              create a
              <RouterLink
                to="/tokens"
                class="underline underline-offset-2 hover:text-foreground transition-colors"
              >
                personal API token
              </RouterLink>
              and use that as the password.
            </p>
          </CardContent>
        </Card>
      </TabsContent>
    </Tabs>
  </div>
</template>
