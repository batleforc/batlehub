<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { ref, onMounted } from "vue";
import { useAuthFetch } from "@/composables/useAuthFetch";
import { extractMessage } from "@/composables/useApi";
import { API_BASE_URL } from "@/config";
import SectionTabs from "@/components/admin/SectionTabs.vue";
import { OPERATIONS_TABS } from "@/config/adminSections";
import { PageHeader } from "@/components/ui/page-header";
import { AsyncState } from "@/components/ui/async-state";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";

const { t } = useI18n();

interface WarmableRegistry {
  name: string;
  latest_n: number;
  concurrency: number;
}

interface WarmResult {
  warmed: number;
  skipped: number;
  errors: number;
}

const { authFetch } = useAuthFetch();

const registries = ref<WarmableRegistry[]>([]);
const loading = ref(false);
const loadError = ref<string | null>(null);

// Per-registry form state
const packageInputs = ref<Record<string, string>>({});
const pathInputs = ref<Record<string, string>>({});
const warming = ref<Record<string, boolean>>({});
const results = ref<Record<string, WarmResult>>({});
const errors = ref<Record<string, string>>({});

async function loadStatus() {
  loading.value = true;
  loadError.value = null;
  try {
    const res = await authFetch(`${API_BASE_URL}/api/v1/admin/warming`);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const body = (await res.json()) as { registries: WarmableRegistry[] };
    registries.value = body.registries;
  } catch (e) {
    loadError.value = extractMessage(e);
  } finally {
    loading.value = false;
  }
}

async function triggerWarm(name: string) {
  warming.value[name] = true;
  delete results.value[name];
  delete errors.value[name];

  const pkgRaw = (packageInputs.value[name] ?? "").trim();
  const pathRaw = (pathInputs.value[name] ?? "").trim();

  const packages = pkgRaw
    ? pkgRaw
        .split(/[\n,]+/)
        .map((s) => s.trim())
        .filter(Boolean)
    : [];
  const paths = pathRaw
    ? pathRaw
        .split(/[\n,]+/)
        .map((s) => s.trim())
        .filter(Boolean)
    : [];

  if (packages.length === 0 && paths.length === 0) {
    errors.value[name] = "Specify at least one package or path.";
    warming.value[name] = false;
    return;
  }

  try {
    const res = await authFetch(
      `${API_BASE_URL}/api/v1/admin/registries/${encodeURIComponent(name)}/warm`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ packages, paths }),
      },
    );
    if (!res.ok) {
      const body = (await res.json().catch(() => ({}))) as { error?: string };
      throw new Error(body.error ?? `HTTP ${res.status}`);
    }
    results.value[name] = (await res.json()) as WarmResult;
  } catch (e) {
    errors.value[name] = extractMessage(e);
  } finally {
    warming.value[name] = false;
  }
}

// ── Delete cached artifact ────────────────────────────────────────────────────

// The delete-cached-artifact card moved to `/admin/observability/health`
// (RFC 0004 Phase 5, *split*): it read nothing this page produces — not the
// warmable-registry list, not `latest_n`, not `concurrency` — and it is the
// opposite verb on a different object. Health already owned "what has this
// registry cached" and the sibling Clear Cache control.

onMounted(() => void loadStatus());
</script>

<template>
  <div class="space-y-6">
    <SectionTabs :tabs="OPERATIONS_TABS" />
    <PageHeader
      variant="display"
      :title="t('adminWarming.cacheWarming')"
      :description="t('adminWarming.registriesWithWarmingConfiguredTrigger')"
    >
      <template #actions>
        <Button variant="outline" size="sm" :disabled="loading" @click="loadStatus">
          {{ loading ? t("common.loading") : t("common.refresh") }}
        </Button>
      </template>
    </PageHeader>

    <AsyncState
      :loading="loading && registries.length === 0"
      :error="loadError"
      :empty="registries.length === 0"
    >
      <template #empty>
        <p class="text-sm text-muted-foreground">
          <i18n-t keypath="adminWarming.noWarmingConfigured" tag="span">
            <template #packages><code>warm_packages</code></template>
            <template #paths><code>warm_paths</code></template>
          </i18n-t>
        </p>
      </template>

      <div class="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
        <Card v-for="reg in registries" :key="reg.name">
          <CardHeader class="pb-2">
            <CardTitle class="text-base font-mono">{{ reg.name }}</CardTitle>
            <div class="flex gap-2 text-xs text-muted-foreground mt-1">
              <span>latest_n: {{ reg.latest_n }}</span>
              <span>·</span>
              <span>concurrency: {{ reg.concurrency }}</span>
            </div>
          </CardHeader>
          <CardContent class="space-y-3">
            <div class="space-y-1.5">
              <Label :for="`pkg-${reg.name}`" class="text-xs">{{ t("common.packages") }}</Label>
              <Input
                :id="`pkg-${reg.name}`"
                v-model="packageInputs[reg.name]"
                :placeholder="t('adminWarming.lodashReact180')"
                class="font-mono text-xs"
              />
              <p class="text-xs text-muted-foreground">
                {{ t("adminWarming.commaSeparatedOmitVersion") }}
              </p>
            </div>
            <div class="space-y-1.5">
              <Label :for="`path-${reg.name}`" class="text-xs">{{ t("common.paths") }}</Label>
              <Input
                :id="`path-${reg.name}`"
                v-model="pathInputs[reg.name]"
                placeholder="idea/idea-2026.1.3.tar.gz"
                class="font-mono text-xs"
              />
              <p class="text-xs text-muted-foreground">
                {{ t("adminWarming.commaSeparatedForPath") }}
              </p>
            </div>

            <p v-if="errors[reg.name]" class="text-xs text-destructive">{{ errors[reg.name] }}</p>

            <div v-if="results[reg.name]" class="flex gap-2 flex-wrap">
              <Badge class="text-xs"> {{ results[reg.name].warmed }} warmed </Badge>
              <Badge class="bg-muted text-muted-foreground text-xs">
                {{ results[reg.name].skipped }} skipped
              </Badge>
              <Badge
                :class="
                  results[reg.name].errors > 0
                    ? 'text-destructive'
                    : 'bg-muted text-muted-foreground'
                "
                class="text-xs"
              >
                {{ results[reg.name].errors }} errors
              </Badge>
            </div>

            <Button size="sm" :disabled="warming[reg.name]" @click="triggerWarm(reg.name)">
              {{ warming[reg.name] ? t("adminWarming.warming") : t("adminWarming.warmNow") }}
            </Button>
          </CardContent>
        </Card>
      </div>
    </AsyncState>

  </div>
</template>
