<script setup lang="ts">
import { listBetaMembers, addBetaMember, removeBetaMember } from "@/client/sdk.gen";
import type { BetaChannelMemberDto } from "@/lib/registry-types";
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

interface AddMemberForm {
  principal_type: string;
  principal_id: string;
  granted_by: string;
}

const {
  registryOptions,
  selectedRegistry,
  items: members,
  itemsError: membersError,
  itemsLoading: membersLoading,
  reloadItems: reloadMembers,
  addDialogOpen,
  addForm,
  addLoading,
  addError,
  submitAdd,
  removeTarget,
  removeLoading,
  removeError,
  confirmRemove,
} = useAdminCrudList<BetaChannelMemberDto, AddMemberForm>({
  listFn: (registry) =>
    listBetaMembers({ path: { registry } }) as Promise<{ data?: unknown; error?: unknown }>,
  addFn: (registry, form) =>
    addBetaMember({
      path: { registry },
      body: {
        principal_type: form.principal_type,
        principal_id: form.principal_id.trim(),
        granted_by: form.granted_by.trim() || undefined,
      },
    }) as Promise<{ data?: unknown; error?: unknown }>,
  removeFn: (registry, item) =>
    removeBetaMember({
      path: { registry, principal_type: item.principal_type, principal_id: item.principal_id },
    }) as Promise<{ data?: unknown; error?: unknown }>,
  initialAddForm: () => ({ principal_type: "user", principal_id: "", granted_by: "" }),
  canSubmitAdd: (form) => !!form.principal_id.trim(),
});

const principalTypeOptions = [
  { value: "user", label: "User" },
  { value: "group", label: "Group" },
];
</script>

<template>
  <div class="space-y-6">
    <SectionTabs :tabs="NAMESPACES_TABS" />
    <PageHeader
      title="Beta Channel"
      description="Manage who can access pre-release versions in each registry."
    >
      <template #actions>
        <Button size="sm" :disabled="!selectedRegistry" @click="addDialogOpen = true">
          Add member
        </Button>
      </template>
    </PageHeader>

    <!-- Registry selector -->
    <div class="space-y-1.5 max-w-xs">
      <Label for="beta-registry">Registry</Label>
      <Select
        id="beta-registry"
        v-model="selectedRegistry"
        placeholder="Select a registry…"
        :options="registryOptions"
      />
    </div>

    <!-- Members table -->
    <Card>
      <CardHeader>
        <div class="flex items-center justify-between">
          <CardTitle class="text-base">
            Members
            <span v-if="selectedRegistry" class="font-mono text-muted-foreground text-sm ml-1"
              >({{ selectedRegistry }})</span
            >
          </CardTitle>
          <Button variant="outline" size="sm" :disabled="membersLoading" @click="reloadMembers">
            {{ membersLoading ? "Loading…" : "Refresh" }}
          </Button>
        </div>
      </CardHeader>
      <CardContent class="p-0">
        <p v-if="membersError" class="p-4 text-sm text-destructive">
          {{ membersError }}
        </p>
        <Table v-else>
          <TableHeader>
            <TableRow>
              <TableHead>Type</TableHead>
              <TableHead>Principal ID</TableHead>
              <TableHead>Granted by</TableHead>
              <TableHead class="text-right"> Actions </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow v-for="m in members" :key="m.principal_type + ':' + m.principal_id">
              <TableCell>
                <Badge
                  :variant="m.principal_type === 'user' ? 'default' : 'secondary'"
                  class="text-xs capitalize"
                >
                  {{ m.principal_type }}
                </Badge>
              </TableCell>
              <TableCell class="font-mono text-sm">
                {{ m.principal_id }}
              </TableCell>
              <TableCell class="text-sm text-muted-foreground">
                {{ m.granted_by ?? "—" }}
              </TableCell>
              <TableCell class="text-right">
                <Button variant="outline" size="sm" @click="removeTarget = m"> Remove </Button>
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
        <p
          v-if="!membersError && (!members || members.length === 0)"
          class="p-6 text-sm text-muted-foreground text-center"
        >
          {{
            selectedRegistry
              ? "No beta channel members for this registry."
              : "Select a registry to view members."
          }}
        </p>
      </CardContent>
    </Card>
  </div>

  <!-- Add member dialog -->
  <Dialog
    :open="addDialogOpen"
    @update:open="
      (v) => {
        if (!v) {
          addDialogOpen = false;
          addError = null;
        }
      }
    "
  >
    <template #title>Add beta channel member</template>
    <template #description>
      Add a user or group to the beta channel for
      <span class="font-mono">{{ selectedRegistry }}</span
      >.
    </template>
    <div class="space-y-4">
      <div class="space-y-3">
        <div class="space-y-1.5">
          <Label for="beta-principal-type">Type</Label>
          <Select
            id="beta-principal-type"
            v-model="addForm.principal_type"
            :options="principalTypeOptions"
          />
        </div>
        <div class="space-y-1.5">
          <Label for="beta-principal-id"
            >Principal ID <span class="text-destructive">*</span></Label
          >
          <Input
            id="beta-principal-id"
            v-model="addForm.principal_id"
            placeholder="e.g. alice or team-frontend"
            class="font-mono"
          />
        </div>
        <div class="space-y-1.5">
          <Label for="beta-granted-by">Granted by</Label>
          <Input
            id="beta-granted-by"
            v-model="addForm.granted_by"
            placeholder="Optional — your user ID"
          />
        </div>
      </div>
      <p v-if="addError" class="text-sm text-destructive">
        {{ addError }}
      </p>
      <div class="flex justify-end gap-2">
        <Button
          variant="outline"
          size="sm"
          :disabled="addLoading"
          @click="
            addDialogOpen = false;
            addError = null;
          "
        >
          Cancel
        </Button>
        <Button size="sm" :disabled="addLoading || !addForm.principal_id.trim()" @click="submitAdd">
          {{ addLoading ? "Adding…" : "Add member" }}
        </Button>
      </div>
    </div>
  </Dialog>

  <!-- Remove confirmation dialog -->
  <Dialog
    :open="removeTarget !== null"
    @update:open="
      (v) => {
        if (!v) {
          removeTarget = null;
          removeError = null;
        }
      }
    "
  >
    <template #title>Remove member?</template>
    <template #description>
      <span class="font-mono">{{ removeTarget?.principal_id }}</span>
      will lose access to pre-release versions in
      <span class="font-mono">{{ selectedRegistry }}</span
      >.
    </template>
    <div class="space-y-4">
      <p v-if="removeError" class="text-sm text-destructive">
        {{ removeError }}
      </p>
      <div class="flex justify-end gap-2">
        <Button
          variant="outline"
          size="sm"
          :disabled="removeLoading"
          @click="
            removeTarget = null;
            removeError = null;
          "
        >
          Cancel
        </Button>
        <Button variant="destructive" size="sm" :disabled="removeLoading" @click="confirmRemove">
          {{ removeLoading ? "Removing…" : "Remove" }}
        </Button>
      </div>
    </div>
  </Dialog>
</template>
