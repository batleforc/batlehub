<script setup lang="ts">
import { ref, watch } from "vue";
import { getPackageVisibility, setPackageVisibility } from "@/client/sdk.gen";
import type { Visibility } from "@/lib/registry-types";
import { VISIBILITY_OPTIONS } from "@/lib/registry-types";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardHeader, CardTitle, CardContent, CardDescription } from "@/components/ui/card";
import Select from "@/components/ui/select/Select.vue";

const props = defineProps<{ registry: string; name: string }>();

const { token } = useAuth();

const { data: visibilityData, reload } = useApi<{ visibility: Visibility }>(
  () => {
    if (!props.registry || !props.name) return Promise.resolve({ data: undefined });
    return getPackageVisibility({ path: { registry: props.registry, name: props.name } }) as Promise<{ data?: unknown; error?: unknown }>;
  },
  [token],
);

const selected = ref<Visibility>("public");
watch(visibilityData, v => { if (v) selected.value = v.visibility; });

const saving = ref(false);
const error = ref<string | null>(null);

async function save() {
  saving.value = true;
  error.value = null;
  try {
    const { error: apiErr } = await setPackageVisibility({
      path: { registry: props.registry, name: props.name },
      body: { visibility: selected.value },
    });
    if (apiErr) throw new Error((apiErr as { message?: string })?.message ?? "API error");
    reload();
  } catch (e) {
    error.value = e instanceof Error ? e.message : "Unknown error";
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <Card>
    <CardHeader>
      <CardTitle class="text-base">Package visibility</CardTitle>
      <CardDescription>Controls who can download this package (all versions share the same setting).</CardDescription>
    </CardHeader>
    <CardContent>
      <div class="flex items-center gap-3 flex-wrap">
        <Badge
          :variant="selected === 'public' ? 'default' : selected === 'internal' ? 'secondary' : 'outline'"
          :class="selected === 'team' ? 'border-primary text-primary' : ''"
          class="capitalize text-xs"
        >
          {{ visibilityData?.visibility ?? "public" }}
        </Badge>
        <Select v-model="selected" :options="[...VISIBILITY_OPTIONS]" class="w-72" />
        <Button
          size="sm"
          :disabled="saving || selected === (visibilityData?.visibility ?? 'public')"
          @click="save"
        >
          {{ saving ? "Saving…" : "Save" }}
        </Button>
      </div>
      <p v-if="error" class="mt-2 text-sm text-destructive">{{ error }}</p>
    </CardContent>
  </Card>
</template>
