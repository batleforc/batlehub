<script setup lang="ts">
import { ref, onMounted } from "vue";
import { useAuthFetch } from "@/composables/useAuthFetch";
import { extractMessage } from "@/composables/useApi";
import { API_BASE_URL } from "@/config";
import SectionTabs from "@/components/admin/SectionTabs.vue";
import { OPERATIONS_TABS } from "@/config/adminSections";
import { PageHeader } from "@/components/ui/page-header";
import { AsyncState } from "@/components/ui/async-state";
import { Card, CardHeader, CardTitle, CardContent, CardDescription } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";

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

type DeleteMode = "package" | "path";

const deleteRegistry = ref("");
const deleteMode = ref<DeleteMode>("package");
const deleteName = ref("");
const deleteVersion = ref("");
const deletePath = ref("");
const deleting = ref(false);
const deleteResult = ref<{ deleted: boolean; artifact_key: string } | null>(null);
const deleteError = ref<string | null>(null);

async function triggerDelete() {
  deleteResult.value = null;
  deleteError.value = null;

  if (!deleteRegistry.value.trim()) {
    deleteError.value = "Registry name is required.";
    return;
  }

  const body: Record<string, string> =
    deleteMode.value === "path"
      ? { path: deletePath.value.trim() }
      : { name: deleteName.value.trim(), version: deleteVersion.value.trim() };

  deleting.value = true;
  try {
    const res = await authFetch(
      `${API_BASE_URL}/api/v1/admin/registries/${encodeURIComponent(deleteRegistry.value.trim())}/cache`,
      {
        method: "DELETE",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      },
    );
    const json = (await res.json().catch(() => ({}))) as {
      deleted?: boolean;
      artifact_key?: string;
      error?: string;
    };
    if (!res.ok) throw new Error(json.error ?? `HTTP ${res.status}`);
    deleteResult.value = { deleted: json.deleted ?? false, artifact_key: json.artifact_key ?? "" };
  } catch (e) {
    deleteError.value = extractMessage(e);
  } finally {
    deleting.value = false;
  }
}

onMounted(() => void loadStatus());
</script>

<template>
  <div class="space-y-6">
    <SectionTabs :tabs="OPERATIONS_TABS" />
    <PageHeader
      title="Cache Warming"
      description="Registries with warming configured. Trigger a warm run to pre-fetch artifacts into the local cache."
    >
      <template #actions>
        <Button variant="outline" size="sm" :disabled="loading" @click="loadStatus">
          {{ loading ? "Loading…" : "Refresh" }}
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
          No registries have warming configured. Add <code>warm_packages</code> or
          <code>warm_paths</code> to a registry in your config.
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
              <Label :for="`pkg-${reg.name}`" class="text-xs">Packages</Label>
              <Input
                :id="`pkg-${reg.name}`"
                v-model="packageInputs[reg.name]"
                placeholder="lodash, react@18.0.0"
                class="font-mono text-xs"
              />
              <p class="text-[11px] text-muted-foreground">
                Comma-separated. Omit version to warm latest_n.
              </p>
            </div>
            <div class="space-y-1.5">
              <Label :for="`path-${reg.name}`" class="text-xs">Paths</Label>
              <Input
                :id="`path-${reg.name}`"
                v-model="pathInputs[reg.name]"
                placeholder="idea/ideaIC-2024.1.4.tar.gz"
                class="font-mono text-xs"
              />
              <p class="text-[11px] text-muted-foreground">
                Comma-separated. For path-addressed registries.
              </p>
            </div>

            <p v-if="errors[reg.name]" class="text-xs text-destructive">{{ errors[reg.name] }}</p>

            <div v-if="results[reg.name]" class="flex gap-2 flex-wrap">
              <Badge class="bg-primary/10 text-primary text-xs">
                {{ results[reg.name].warmed }} warmed
              </Badge>
              <Badge class="bg-muted text-muted-foreground text-xs">
                {{ results[reg.name].skipped }} skipped
              </Badge>
              <Badge
                :class="
                  results[reg.name].errors > 0
                    ? 'bg-destructive/10 text-destructive'
                    : 'bg-muted text-muted-foreground'
                "
                class="text-xs"
              >
                {{ results[reg.name].errors }} errors
              </Badge>
            </div>

            <Button size="sm" :disabled="warming[reg.name]" @click="triggerWarm(reg.name)">
              {{ warming[reg.name] ? "Warming…" : "Warm Now" }}
            </Button>
          </CardContent>
        </Card>
      </div>
    </AsyncState>

    <!-- Delete cached artifact -->
    <Card>
      <CardHeader>
        <CardTitle>Delete Cached Artifact</CardTitle>
        <CardDescription>
          Remove a single proxy-cached artifact from storage. The next request will re-download it
          from upstream. Use <strong>Path</strong> mode for path-addressed registries
          (jetbrains/deb/rpm); use <strong>Package</strong> mode for all others.
        </CardDescription>
      </CardHeader>
      <CardContent class="space-y-4">
        <!-- Registry -->
        <div class="space-y-1.5">
          <Label for="del-registry" class="text-xs">Registry</Label>
          <Input
            id="del-registry"
            v-model="deleteRegistry"
            placeholder="e.g. npm, jetbrains-ide"
            class="font-mono text-sm max-w-xs"
          />
        </div>

        <!-- Mode toggle -->
        <div class="flex gap-2">
          <button
            :class="[
              'px-3 py-1 rounded-sm text-xs font-mono border transition-colors',
              deleteMode === 'package'
                ? 'bg-primary text-primary-foreground border-primary'
                : 'border-border text-muted-foreground hover:bg-accent',
            ]"
            @click="deleteMode = 'package'"
          >
            Package
          </button>
          <button
            :class="[
              'px-3 py-1 rounded-sm text-xs font-mono border transition-colors',
              deleteMode === 'path'
                ? 'bg-primary text-primary-foreground border-primary'
                : 'border-border text-muted-foreground hover:bg-accent',
            ]"
            @click="deleteMode = 'path'"
          >
            Path
          </button>
        </div>

        <!-- Package mode -->
        <div v-if="deleteMode === 'package'" class="flex gap-3 flex-wrap">
          <div class="space-y-1.5">
            <Label for="del-name" class="text-xs">Name</Label>
            <Input
              id="del-name"
              v-model="deleteName"
              placeholder="lodash"
              class="font-mono text-sm w-44"
            />
          </div>
          <div class="space-y-1.5">
            <Label for="del-version" class="text-xs">Version</Label>
            <Input
              id="del-version"
              v-model="deleteVersion"
              placeholder="4.17.21"
              class="font-mono text-sm w-36"
            />
          </div>
        </div>

        <!-- Path mode -->
        <div v-else class="space-y-1.5">
          <Label for="del-path" class="text-xs">Path</Label>
          <Input
            id="del-path"
            v-model="deletePath"
            placeholder="idea/ideaIC-2026.1.3.tar.gz"
            class="font-mono text-sm max-w-sm"
          />
        </div>

        <!-- Feedback -->
        <p v-if="deleteError" class="text-xs text-destructive">{{ deleteError }}</p>
        <div v-if="deleteResult" class="space-y-1">
          <Badge
            :class="
              deleteResult.deleted ? 'bg-primary/10 text-primary' : 'bg-muted text-muted-foreground'
            "
            class="text-xs"
          >
            {{ deleteResult.deleted ? "Deleted" : "Not cached — nothing to remove" }}
          </Badge>
          <p class="text-[11px] text-muted-foreground font-mono break-all">
            {{ deleteResult.artifact_key }}
          </p>
        </div>

        <Button variant="destructive" size="sm" :disabled="deleting" @click="triggerDelete">
          {{ deleting ? "Deleting…" : "Delete from Cache" }}
        </Button>
      </CardContent>
    </Card>
  </div>
</template>
