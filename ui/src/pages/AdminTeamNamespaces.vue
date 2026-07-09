<script setup lang="ts">
import { listNamespaces, claimNamespace, releaseNamespace } from "@/client/sdk.gen";
import type { TeamNamespaceDto } from "@/lib/registry-types";
import { useAdminCrudList } from "@/composables/useAdminCrudList";
import SectionTabs from "@/components/admin/SectionTabs.vue";
import { NAMESPACES_TABS } from "@/config/adminSections";
import { PageHeader } from "@/components/ui/page-header";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import { Dialog } from "@/components/ui/dialog";

interface ClaimForm {
  prefix: string;
  group_id: string;
  claimed_by: string;
}

const {
  registryOptions,
  selectedRegistry,
  items: namespaces,
  itemsError: namespacesError,
  itemsLoading: namespacesLoading,
  reloadItems: reloadNamespaces,
  addDialogOpen: claimDialogOpen,
  addForm: claimForm,
  addLoading: claimLoading,
  addError: claimError,
  submitAdd: submitClaim,
  removeTarget: releaseTarget,
  removeLoading: releaseLoading,
  removeError: releaseError,
  confirmRemove: confirmRelease,
} = useAdminCrudList<TeamNamespaceDto, ClaimForm>({
  listFn: (registry) =>
    listNamespaces({ path: { registry } }) as Promise<{ data?: unknown; error?: unknown }>,
  addFn: (registry, form) =>
    claimNamespace({
      path: { registry },
      body: {
        prefix: form.prefix.trim(),
        group_id: form.group_id.trim(),
        claimed_by: form.claimed_by.trim() || undefined,
      },
    }) as Promise<{ data?: unknown; error?: unknown }>,
  // The prefix may contain slashes — passed verbatim; the backend route uses {prefix:.*}
  removeFn: (registry, item) =>
    releaseNamespace({
      path: { registry, prefix: item.prefix },
    }) as Promise<{ data?: unknown; error?: unknown }>,
  initialAddForm: () => ({ prefix: "", group_id: "", claimed_by: "" }),
  canSubmitAdd: (form) => !!form.prefix.trim() && !!form.group_id.trim(),
});
</script>

<template>
  <div class="space-y-6">
    <SectionTabs :tabs="NAMESPACES_TABS" />
    <PageHeader
      title="Team Namespaces"
      description="Assign package name prefixes to auth-provider groups to control who may publish within them."
    >
      <template #actions>
        <Button size="sm" :disabled="!selectedRegistry" @click="claimDialogOpen = true">
          Claim namespace
        </Button>
      </template>
    </PageHeader>

    <!-- Registry selector -->
    <div class="space-y-1.5 max-w-xs">
      <Label for="team-ns-registry">Registry</Label>
      <Select
        id="team-ns-registry"
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
            <span v-if="selectedRegistry" class="font-mono text-muted-foreground text-sm ml-1"
              >({{ selectedRegistry }})</span
            >
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
        <p v-if="namespacesError" class="p-4 text-sm text-destructive">
          {{ namespacesError }}
        </p>
        <Table v-else>
          <TableHeader>
            <TableRow>
              <TableHead>Prefix</TableHead>
              <TableHead>Group</TableHead>
              <TableHead>Claimed by</TableHead>
              <TableHead class="text-right"> Actions </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow v-for="ns in namespaces" :key="ns.registry + ':' + ns.prefix">
              <TableCell class="font-mono text-sm">
                {{ ns.prefix }}
              </TableCell>
              <TableCell>
                <Badge variant="secondary" class="text-xs font-mono">
                  {{ ns.group_id }}
                </Badge>
              </TableCell>
              <TableCell class="text-sm text-muted-foreground">
                {{ ns.claimed_by ?? "—" }}
              </TableCell>
              <TableCell class="text-right">
                <Button variant="outline" size="sm" @click="releaseTarget = ns"> Release </Button>
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
        <p
          v-if="!namespacesError && (!namespaces || namespaces.length === 0)"
          class="p-6 text-sm text-muted-foreground text-center"
        >
          {{
            selectedRegistry
              ? "No namespace claims for this registry."
              : "Select a registry to view namespace claims."
          }}
        </p>
      </CardContent>
    </Card>
  </div>

  <!-- Claim namespace dialog -->
  <Dialog
    :open="claimDialogOpen"
    @update:open="
      (v) => {
        if (!v) {
          claimDialogOpen = false;
          claimError = null;
        }
      }
    "
  >
    <template #title>Claim namespace</template>
    <template #description>
      Restrict publishing under a prefix in
      <span class="font-mono">{{ selectedRegistry }}</span> to a specific group.
    </template>
    <div class="space-y-4">
      <div class="space-y-3">
        <div class="space-y-1.5">
          <Label for="team-ns-prefix">Prefix <span class="text-destructive">*</span></Label>
          <Input
            id="team-ns-prefix"
            v-model="claimForm.prefix"
            placeholder="e.g. frontend or frontend/ui"
            class="font-mono"
          />
          <p class="text-xs text-muted-foreground">
            Packages whose name equals or starts with <span class="font-mono">prefix/</span> will be
            restricted.
          </p>
        </div>
        <div class="space-y-1.5">
          <Label for="team-ns-group-id">Group ID <span class="text-destructive">*</span></Label>
          <Input
            id="team-ns-group-id"
            v-model="claimForm.group_id"
            placeholder="e.g. oidc:frontend-team"
            class="font-mono"
          />
          <p class="text-xs text-muted-foreground">
            Must match the group name in your auth provider's claims.
          </p>
        </div>
        <div class="space-y-1.5">
          <Label for="team-ns-claimed-by">Claimed by</Label>
          <Input
            id="team-ns-claimed-by"
            v-model="claimForm.claimed_by"
            placeholder="Optional — your user ID"
          />
        </div>
      </div>
      <p v-if="claimError" class="text-sm text-destructive">
        {{ claimError }}
      </p>
      <div class="flex justify-end gap-2">
        <Button
          variant="outline"
          size="sm"
          :disabled="claimLoading"
          @click="
            claimDialogOpen = false;
            claimError = null;
          "
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
    @update:open="
      (v) => {
        if (!v) {
          releaseTarget = null;
          releaseError = null;
        }
      }
    "
  >
    <template #title>Release namespace claim?</template>
    <template #description>
      The prefix <span class="font-mono">{{ releaseTarget?.prefix }}</span> will no longer be
      restricted to group <span class="font-mono">{{ releaseTarget?.group_id }}</span
      >. Any authenticated user will be able to publish packages under this prefix.
    </template>
    <div class="space-y-4">
      <p v-if="releaseError" class="text-sm text-destructive">
        {{ releaseError }}
      </p>
      <div class="flex justify-end gap-2">
        <Button
          variant="outline"
          size="sm"
          :disabled="releaseLoading"
          @click="
            releaseTarget = null;
            releaseError = null;
          "
        >
          Cancel
        </Button>
        <Button variant="destructive" size="sm" :disabled="releaseLoading" @click="confirmRelease">
          {{ releaseLoading ? "Releasing…" : "Release claim" }}
        </Button>
      </div>
    </div>
  </Dialog>
</template>
