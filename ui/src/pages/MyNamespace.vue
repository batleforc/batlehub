<script setup lang="ts">
import { ref, computed, watch } from "vue";
import { useAuth } from "@/composables/useAuth";
import { useApi } from "@/composables/useApi";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardHeader, CardTitle, CardContent, CardDescription } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import Select from "@/components/ui/select/Select.vue";

const { token, identity } = useAuth();
const API_BASE = import.meta.env.VITE_API_BASE_URL ?? "";

type Visibility = "public" | "internal" | "team";

interface RegistryInfo {
  name: string;
  type: string;
}

// ── Registries ────────────────────────────────────────────────────────────────

const { data: registriesData } = useApi<RegistryInfo[]>(
  () =>
    fetch(`${API_BASE}/api/v1/registries`, {
      headers: token.value ? { Authorization: `Bearer ${token.value}` } : {},
    }).then(async (r) => {
      if (!r.ok) throw new Error(await r.text());
      return { data: await r.json() };
    }) as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

const registryOptions = computed(() =>
  (registriesData.value ?? []).map((r) => ({ value: r.name, label: r.name })),
);

const selectedRegistry = ref("");
watch(registriesData, (list) => {
  if (list && list.length > 0 && !selectedRegistry.value) {
    selectedRegistry.value = list[0].name;
  }
});

// ── Package lookup ────────────────────────────────────────────────────────────

const packageName = ref("");
const lookupTrigger = ref(0);

const {
  data: visibilityData,
  error: visibilityError,
  loading: visibilityLoading,
  reload: reloadVisibility,
} = useApi<{ visibility: Visibility }>(
  () => {
    if (!selectedRegistry.value || !packageName.value.trim()) {
      return Promise.resolve({ data: undefined }) as Promise<{ data?: unknown; error?: unknown }>;
    }
    // Touch trigger so manual lookups fire.
    void lookupTrigger.value;
    return fetch(
      `${API_BASE}/api/v1/admin/registries/${encodeURIComponent(selectedRegistry.value)}/packages/${packageName.value.trim()}/visibility`,
      { headers: token.value ? { Authorization: `Bearer ${token.value}` } : {} },
    ).then(async (r) => {
      if (!r.ok) throw new Error(await r.text());
      return { data: await r.json() };
    }) as Promise<{ data?: unknown; error?: unknown }>;
  },
  [token, selectedRegistry, lookupTrigger],
);

const selectedVisibility = ref<Visibility>("public");
watch(visibilityData, (v) => { if (v) selectedVisibility.value = v.visibility; });

function lookup() {
  if (!packageName.value.trim() || !selectedRegistry.value) return;
  lookupTrigger.value++;
}

// ── Save visibility ───────────────────────────────────────────────────────────

const saveLoading = ref(false);
const saveError = ref<string | null>(null);
const saveSuccess = ref(false);

async function saveVisibility() {
  if (!selectedRegistry.value || !packageName.value.trim()) return;
  saveLoading.value = true;
  saveError.value = null;
  saveSuccess.value = false;
  try {
    const r = await fetch(
      `${API_BASE}/api/v1/admin/registries/${encodeURIComponent(selectedRegistry.value)}/packages/${packageName.value.trim()}/visibility`,
      {
        method: "PUT",
        headers: {
          "Content-Type": "application/json",
          ...(token.value ? { Authorization: `Bearer ${token.value}` } : {}),
        },
        body: JSON.stringify({ visibility: selectedVisibility.value }),
      },
    );
    if (!r.ok) throw new Error(await r.text());
    saveSuccess.value = true;
    reloadVisibility();
    setTimeout(() => { saveSuccess.value = false; }, 3000);
  } catch (e) {
    saveError.value = e instanceof Error ? e.message : "Unknown error";
  } finally {
    saveLoading.value = false;
  }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

const visibilityOptions = [
  { value: "public",   label: "Public — anyone can download" },
  { value: "internal", label: "Internal — authenticated users only" },
  { value: "team",     label: "Team — namespace group members only" },
];

const visibilityVariant = computed(() => {
  if (selectedVisibility.value === "public") return "default";
  if (selectedVisibility.value === "internal") return "secondary";
  return "outline";
});

const groups = computed(() => identity.value?.groups ?? []);
const hasGroups = computed(() => groups.value.length > 0);
</script>

<template>
  <div class="space-y-6 max-w-2xl">
    <!-- Header -->
    <div>
      <h1 class="text-2xl font-semibold">My Namespace</h1>
      <p class="text-sm text-muted-foreground mt-0.5">
        Manage visibility for packages within your team namespaces.
      </p>
    </div>

    <!-- Groups summary -->
    <Card>
      <CardHeader>
        <CardTitle class="text-base">Your groups</CardTitle>
        <CardDescription>
          You can manage packages whose namespace prefix is owned by one of these groups.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div v-if="hasGroups" class="flex flex-wrap gap-2">
          <Badge
            v-for="g in groups"
            :key="g"
            variant="secondary"
            class="font-mono text-xs"
          >
            {{ g }}
          </Badge>
        </div>
        <p v-else class="text-sm text-muted-foreground">
          You are not a member of any groups. Contact your administrator to be added to a team namespace.
        </p>
      </CardContent>
    </Card>

    <!-- Package visibility manager -->
    <Card>
      <CardHeader>
        <CardTitle class="text-base">Package visibility</CardTitle>
        <CardDescription>
          Enter a package name to view and change its visibility. You must be a member of the group
          that owns the package's namespace prefix.
        </CardDescription>
      </CardHeader>
      <CardContent class="space-y-4">
        <!-- Registry + package input -->
        <div class="flex flex-col sm:flex-row gap-3">
          <div class="space-y-1.5 w-full sm:w-44 shrink-0">
            <Label>Registry</Label>
            <Select
              v-model="selectedRegistry"
              placeholder="Select registry…"
              :options="registryOptions"
            />
          </div>
          <div class="space-y-1.5 flex-1">
            <Label>Package name</Label>
            <div class="flex gap-2">
              <Input
                v-model="packageName"
                placeholder="e.g. frontend/utils"
                class="font-mono"
                @keyup.enter="lookup"
              />
              <Button
                variant="outline"
                size="sm"
                :disabled="!packageName.trim() || !selectedRegistry || visibilityLoading"
                @click="lookup"
              >
                {{ visibilityLoading ? "…" : "Fetch" }}
              </Button>
            </div>
          </div>
        </div>

        <!-- Error from fetch -->
        <p v-if="visibilityError" class="text-sm text-destructive">{{ visibilityError }}</p>

        <!-- Visibility control (shown once we have data) -->
        <template v-if="visibilityData">
          <div class="border rounded-lg p-4 space-y-3">
            <div class="flex items-center gap-2">
              <span class="text-sm font-medium">Current visibility:</span>
              <Badge
                :variant="visibilityVariant"
                :class="selectedVisibility === 'team' ? 'border-blue-500 text-blue-600' : ''"
                class="capitalize text-xs"
              >
                {{ visibilityData.visibility }}
              </Badge>
            </div>

            <div class="flex items-center gap-3 flex-wrap">
              <Select
                v-model="selectedVisibility"
                :options="visibilityOptions"
                class="w-72"
              />
              <Button
                size="sm"
                :disabled="saveLoading || selectedVisibility === visibilityData.visibility"
                @click="saveVisibility"
              >
                {{ saveLoading ? "Saving…" : "Save" }}
              </Button>
            </div>

            <p v-if="saveError" class="text-sm text-destructive">{{ saveError }}</p>
            <p v-if="saveSuccess" class="text-sm text-green-600">Visibility updated.</p>
          </div>
        </template>

        <!-- Guidance when no data yet -->
        <p v-else-if="!visibilityLoading && !visibilityError" class="text-sm text-muted-foreground">
          Enter a registry and package name, then click <strong>Fetch</strong> to load the current visibility.
        </p>
      </CardContent>
    </Card>
  </div>
</template>
