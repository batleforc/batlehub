<script setup lang="ts">
import { ref, computed, watch } from "vue";
import { registryHealth, listBetaMembers, addBetaMember, removeBetaMember } from "@/client/sdk.gen";
import type { RegistryHealthDto } from "@/client/types.gen";
import type { BetaChannelMemberDto } from "@/lib/registry-types";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import Select from "@/components/ui/select/Select.vue";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import Dialog from "@/components/ui/dialog/Dialog.vue";

const { token } = useAuth();

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

const {
  data: members,
  error: membersError,
  loading: membersLoading,
  reload: reloadMembers,
} = useApi<BetaChannelMemberDto[]>(() => {
  if (!selectedRegistry.value) return Promise.resolve({ data: [] });
  return listBetaMembers({ path: { registry: selectedRegistry.value } }) as Promise<{
    data?: unknown;
    error?: unknown;
  }>;
}, [token, selectedRegistry]);

const addDialogOpen = ref(false);
const addForm = ref({ principal_type: "user", principal_id: "", granted_by: "" });
const addLoading = ref(false);
const addError = ref<string | null>(null);

async function submitAdd() {
  if (!addForm.value.principal_id.trim() || !selectedRegistry.value) return;
  addLoading.value = true;
  addError.value = null;
  try {
    const { error: apiErr } = await addBetaMember({
      path: { registry: selectedRegistry.value },
      body: {
        principal_type: addForm.value.principal_type,
        principal_id: addForm.value.principal_id.trim(),
        granted_by: addForm.value.granted_by.trim() || undefined,
      },
    });
    if (apiErr) throw new Error((apiErr as { message?: string })?.message ?? "API error");
    addDialogOpen.value = false;
    addForm.value = { principal_type: "user", principal_id: "", granted_by: "" };
    reloadMembers();
  } catch (e) {
    addError.value = e instanceof Error ? e.message : "Unknown error";
  } finally {
    addLoading.value = false;
  }
}

const removeTarget = ref<BetaChannelMemberDto | null>(null);
const removeLoading = ref(false);
const removeError = ref<string | null>(null);

async function confirmRemove() {
  if (!removeTarget.value || !selectedRegistry.value) return;
  removeLoading.value = true;
  removeError.value = null;
  try {
    const { principal_type, principal_id } = removeTarget.value;
    const { error: apiErr } = await removeBetaMember({
      path: { registry: selectedRegistry.value, principal_type, principal_id },
    });
    if (apiErr) throw new Error((apiErr as { message?: string })?.message ?? "API error");
    removeTarget.value = null;
    reloadMembers();
  } catch (e) {
    removeError.value = e instanceof Error ? e.message : "Unknown error";
  } finally {
    removeLoading.value = false;
  }
}

const principalTypeOptions = [
  { value: "user", label: "User" },
  { value: "group", label: "Group" },
];
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-center justify-between">
      <div>
        <h1 class="text-2xl font-semibold">Beta Channel</h1>
        <p class="text-sm text-muted-foreground mt-0.5">
          Manage who can access pre-release versions in each registry.
        </p>
      </div>
      <Button size="sm" :disabled="!selectedRegistry" @click="addDialogOpen = true">
        Add member
      </Button>
    </div>

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
    <div class="space-y-4">
      <div>
        <h2 class="text-lg font-semibold">Add beta channel member</h2>
        <p class="text-sm text-muted-foreground mt-1">
          Add a user or group to the beta channel for
          <span class="font-mono">{{ selectedRegistry }}</span
          >.
        </p>
      </div>
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
    <div class="space-y-4">
      <div>
        <h2 class="text-lg font-semibold">Remove member?</h2>
        <p class="text-sm text-muted-foreground mt-1">
          <span class="font-mono">{{ removeTarget?.principal_id }}</span>
          will lose access to pre-release versions in
          <span class="font-mono">{{ selectedRegistry }}</span
          >.
        </p>
      </div>
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
