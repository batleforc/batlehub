<script setup lang="ts">
import { ref, computed, watch } from "vue";
import { registryHealth, listNamespaces, claimNamespace, releaseNamespace } from "@/client/sdk.gen";
import type { RegistryHealthDto } from "@/client/types.gen";
import type { TeamNamespaceDto } from "@/lib/registry-types";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import Select from "@/components/ui/select/Select.vue";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell,
} from "@/components/ui/table";
import Dialog from "@/components/ui/dialog/Dialog.vue";

const { token } = useAuth();

// ── Registry selector ─────────────────────────────────────────────────────────

const { data: registriesData } = useApi<RegistryHealthDto[]>(
  () => registryHealth() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

const registryOptions = computed(() =>
  (registriesData.value ?? []).map((r) => ({ value: r.registry, label: r.registry })),
);

const selectedRegistry = ref<string>("");

watch(registriesData, (list) => {
  if (list && list.length > 0 && !selectedRegistry.value) {
    selectedRegistry.value = list[0].registry;
  }
});

// ── Namespace list ────────────────────────────────────────────────────────────

const {
  data: namespaces,
  error: namespacesError,
  loading: namespacesLoading,
  reload: reloadNamespaces,
} = useApi<TeamNamespaceDto[]>(
  () => {
    if (!selectedRegistry.value) return Promise.resolve({ data: [] });
    return listNamespaces({ path: { registry: selectedRegistry.value } }) as Promise<{ data?: unknown; error?: unknown }>;
  },
  [token, selectedRegistry],
);

// ── Claim namespace dialog ────────────────────────────────────────────────────

const claimDialogOpen = ref(false);
const claimForm = ref({ prefix: "", group_id: "", claimed_by: "" });
const claimLoading = ref(false);
const claimError = ref<string | null>(null);

async function submitClaim() {
  if (!claimForm.value.prefix.trim() || !claimForm.value.group_id.trim() || !selectedRegistry.value) return;
  claimLoading.value = true;
  claimError.value = null;
  try {
    const { error: apiErr } = await claimNamespace({
      path: { registry: selectedRegistry.value },
      body: {
        prefix: claimForm.value.prefix.trim(),
        group_id: claimForm.value.group_id.trim(),
        claimed_by: claimForm.value.claimed_by.trim() || undefined,
      },
    });
    if (apiErr) throw new Error((apiErr as { message?: string })?.message ?? "API error");
    claimDialogOpen.value = false;
    claimForm.value = { prefix: "", group_id: "", claimed_by: "" };
    reloadNamespaces();
  } catch (e) {
    claimError.value = e instanceof Error ? e.message : "Unknown error";
  } finally {
    claimLoading.value = false;
  }
}

// ── Release namespace dialog ──────────────────────────────────────────────────

const releaseTarget = ref<TeamNamespaceDto | null>(null);
const releaseLoading = ref(false);
const releaseError = ref<string | null>(null);

async function confirmRelease() {
  if (!releaseTarget.value || !selectedRegistry.value) return;
  releaseLoading.value = true;
  releaseError.value = null;
  try {
    // The prefix may contain slashes — passed verbatim; the backend route uses {prefix:.*}
    const { error: apiErr } = await releaseNamespace({
      path: { registry: selectedRegistry.value, prefix: releaseTarget.value.prefix },
    });
    if (apiErr) throw new Error((apiErr as { message?: string })?.message ?? "API error");
    releaseTarget.value = null;
    reloadNamespaces();
  } catch (e) {
    releaseError.value = e instanceof Error ? e.message : "Unknown error";
  } finally {
    releaseLoading.value = false;
  }
}
</script>

<template>
  <div class="space-y-6">
    <!-- Header -->
    <div class="flex items-center justify-between">
      <div>
        <h1 class="text-2xl font-semibold">
          Team Namespaces
        </h1>
        <p class="text-sm text-muted-foreground mt-0.5">
          Assign package name prefixes to auth-provider groups to control who may publish within them.
        </p>
      </div>
      <Button
        size="sm"
        :disabled="!selectedRegistry"
        @click="claimDialogOpen = true"
      >
        Claim namespace
      </Button>
    </div>

    <!-- Registry selector -->
    <div class="space-y-1.5 max-w-xs">
      <Label>Registry</Label>
      <Select
        v-model="selectedRegistry"
        placeholder="Select a registry…"
        :options="registryOptions"
      />
    </div>

    <!-- Namespaces table -->
    <Card>
      <CardHeader>
        <div class="flex items-center justify-between">
          <CardTitle class="text-base">
            Namespace claims
            <span
              v-if="selectedRegistry"
              class="font-mono text-muted-foreground text-sm ml-1"
            >({{ selectedRegistry }})</span>
          </CardTitle>
          <Button
            variant="outline"
            size="sm"
            :disabled="namespacesLoading"
            @click="reloadNamespaces"
          >
            {{ namespacesLoading ? "Loading…" : "Refresh" }}
          </Button>
        </div>
      </CardHeader>
      <CardContent class="p-0">
        <p
          v-if="namespacesError"
          class="p-4 text-sm text-destructive"
        >
          {{ namespacesError }}
        </p>
        <Table v-else>
          <TableHeader>
            <TableRow>
              <TableHead>Prefix</TableHead>
              <TableHead>Group</TableHead>
              <TableHead>Claimed by</TableHead>
              <TableHead class="text-right">
                Actions
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow
              v-for="ns in namespaces"
              :key="ns.registry + ':' + ns.prefix"
            >
              <TableCell class="font-mono text-sm">
                {{ ns.prefix }}
              </TableCell>
              <TableCell>
                <Badge
                  variant="secondary"
                  class="text-xs font-mono"
                >
                  {{ ns.group_id }}
                </Badge>
              </TableCell>
              <TableCell class="text-sm text-muted-foreground">
                {{ ns.claimed_by ?? "—" }}
              </TableCell>
              <TableCell class="text-right">
                <Button
                  variant="outline"
                  size="sm"
                  @click="releaseTarget = ns"
                >
                  Release
                </Button>
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
        <p
          v-if="!namespacesError && (!namespaces || namespaces.length === 0)"
          class="p-6 text-sm text-muted-foreground text-center"
        >
          {{ selectedRegistry ? "No namespace claims for this registry." : "Select a registry to view namespace claims." }}
        </p>
      </CardContent>
    </Card>
  </div>

  <!-- Claim namespace dialog -->
  <Dialog
    :open="claimDialogOpen"
    @update:open="(v) => { if (!v) { claimDialogOpen = false; claimError = null; } }"
  >
    <div class="space-y-4">
      <div>
        <h2 class="text-lg font-semibold">
          Claim namespace
        </h2>
        <p class="text-sm text-muted-foreground mt-1">
          Restrict publishing under a prefix in
          <span class="font-mono">{{ selectedRegistry }}</span> to a specific group.
        </p>
      </div>
      <div class="space-y-3">
        <div class="space-y-1.5">
          <Label>Prefix <span class="text-destructive">*</span></Label>
          <Input
            v-model="claimForm.prefix"
            placeholder="e.g. frontend or frontend/ui"
            class="font-mono"
          />
          <p class="text-xs text-muted-foreground">
            Packages whose name equals or starts with <span class="font-mono">prefix/</span> will be restricted.
          </p>
        </div>
        <div class="space-y-1.5">
          <Label>Group ID <span class="text-destructive">*</span></Label>
          <Input
            v-model="claimForm.group_id"
            placeholder="e.g. oidc:frontend-team"
            class="font-mono"
          />
          <p class="text-xs text-muted-foreground">
            Must match the group name in your auth provider's claims.
          </p>
        </div>
        <div class="space-y-1.5">
          <Label>Claimed by</Label>
          <Input
            v-model="claimForm.claimed_by"
            placeholder="Optional — your user ID"
          />
        </div>
      </div>
      <p
        v-if="claimError"
        class="text-sm text-destructive"
      >
        {{ claimError }}
      </p>
      <div class="flex justify-end gap-2">
        <Button
          variant="outline"
          size="sm"
          :disabled="claimLoading"
          @click="claimDialogOpen = false; claimError = null"
        >
          Cancel
        </Button>
        <Button
          size="sm"
          :disabled="claimLoading || !claimForm.prefix.trim() || !claimForm.group_id.trim()"
          @click="submitClaim"
        >
          {{ claimLoading ? "Claiming…" : "Claim namespace" }}
        </Button>
      </div>
    </div>
  </Dialog>

  <!-- Release confirmation dialog -->
  <Dialog
    :open="releaseTarget !== null"
    @update:open="(v) => { if (!v) { releaseTarget = null; releaseError = null; } }"
  >
    <div class="space-y-4">
      <div>
        <h2 class="text-lg font-semibold">
          Release namespace claim?
        </h2>
        <p class="text-sm text-muted-foreground mt-1">
          The prefix <span class="font-mono">{{ releaseTarget?.prefix }}</span> will no longer be
          restricted to group <span class="font-mono">{{ releaseTarget?.group_id }}</span>.
          Any authenticated user will be able to publish packages under this prefix.
        </p>
      </div>
      <p
        v-if="releaseError"
        class="text-sm text-destructive"
      >
        {{ releaseError }}
      </p>
      <div class="flex justify-end gap-2">
        <Button
          variant="outline"
          size="sm"
          :disabled="releaseLoading"
          @click="releaseTarget = null; releaseError = null"
        >
          Cancel
        </Button>
        <Button
          variant="destructive"
          size="sm"
          :disabled="releaseLoading"
          @click="confirmRelease"
        >
          {{ releaseLoading ? "Releasing…" : "Release claim" }}
        </Button>
      </div>
    </div>
  </Dialog>
</template>
