<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { listNamespaces, claimNamespace, releaseNamespace } from "@/client/sdk.gen";
import type { TeamNamespaceDto } from "@/client/types.gen";
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

const { t } = useI18n();

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
      variant="display"
      :title="t('adminTeamNamespaces.teamNamespaces')"
      :description="t('adminTeamNamespaces.assignPackageNamePrefixesTo')"
    >
      <template #actions>
        <Button size="sm" :disabled="!selectedRegistry" @click="claimDialogOpen = true">{{
          t("adminTeamNamespaces.claimNamespace")
        }}</Button>
      </template>
    </PageHeader>

    <!-- Registry selector -->
    <div class="space-y-1.5 max-w-xs">
      <Label for="team-ns-registry">{{ t("common.registry") }}</Label>
      <Select
        id="team-ns-registry"
        v-model="selectedRegistry"
        :placeholder="t('adminTeamNamespaces.selectARegistry')"
        :options="registryOptions"
      />
    </div>

    <!-- Namespaces table -->
    <Card>
      <CardHeader>
        <div class="flex items-center justify-between">
          <CardTitle class="text-base">
            {{ t("adminTeamNamespaces.namespaceClaims") }}
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
            {{ namespacesLoading ? t("common.loading") : t("common.refresh") }}
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
              <TableHead>{{ t("common.prefix") }}</TableHead>
              <TableHead>{{ t("common.group") }}</TableHead>
              <TableHead>{{ t("adminTeamNamespaces.claimedBy") }}</TableHead>
              <TableHead class="text-right"> {{ t("common.actions") }} </TableHead>
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
                <Button variant="outline" size="sm" @click="releaseTarget = ns">
                  {{ t("common.release") }}
                </Button>
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
        <p v-if="namespacesLoading" class="p-6 text-center text-sm text-muted-foreground">
          {{ t("common.loading") }}
        </p>
        <!--
          `&& !namespacesLoading`: the empty state rendered while the request was still
          in flight, so an operator read "none for this registry", opened the
          dialog and claimed something that already existed. Empty must mean
          empty, not "we have not looked yet".
        -->
        <p
          v-if="!namespacesError && !namespacesLoading && (!namespaces || namespaces.length === 0)"
          class="p-6 text-sm text-muted-foreground text-center"
        >
          {{
            selectedRegistry
              ? t("adminTeamNamespaces.noNamespaceClaimsForThis")
              : t("adminTeamNamespaces.selectARegistryToView")
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
    <template #title>{{ t("adminTeamNamespaces.claimNamespace") }}</template>
    <template #description>
      <i18n-t keypath="adminTeamNamespaces.restrictPublishingIn" tag="span">
        <template #registry
          ><span class="font-mono">{{ selectedRegistry }}</span></template
        >
      </i18n-t>
    </template>
    <div class="space-y-4">
      <div class="space-y-3">
        <div class="space-y-1.5">
          <Label for="team-ns-prefix">
            {{ t("adminTeamNamespaces.prefix") }} <span class="text-destructive">*</span>
          </Label>
          <Input
            id="team-ns-prefix"
            v-model="claimForm.prefix"
            placeholder="e.g. frontend or frontend/ui"
            class="font-mono"
          />
          <p class="text-xs text-muted-foreground">
            <i18n-t keypath="adminTeamNamespaces.packagesStartingWith" tag="span">
              <template #prefix><span class="font-mono">prefix/</span></template>
            </i18n-t>
          </p>
        </div>
        <div class="space-y-1.5">
          <Label for="team-ns-group-id">
            {{ t("adminTeamNamespaces.groupId") }} <span class="text-destructive">*</span>
          </Label>
          <Input
            id="team-ns-group-id"
            v-model="claimForm.group_id"
            placeholder="e.g. oidc:frontend-team"
            class="font-mono"
          />
          <p class="text-xs text-muted-foreground">
            {{ t("adminTeamNamespaces.mustMatchTheGroup") }}
          </p>
        </div>
        <div class="space-y-1.5">
          <Label for="team-ns-claimed-by">{{ t("adminTeamNamespaces.claimedBy") }}</Label>
          <Input
            id="team-ns-claimed-by"
            v-model="claimForm.claimed_by"
            :placeholder="t('adminTeamNamespaces.optionalYourUserId')"
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
          {{ t("common.cancel") }}
        </Button>
        <Button
          size="sm"
          :disabled="claimLoading || !claimForm.prefix.trim() || !claimForm.group_id.trim()"
          @click="submitClaim"
        >
          {{
            claimLoading
              ? t("adminTeamNamespaces.claiming")
              : t("adminTeamNamespaces.claimNamespace")
          }}
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
    <template #title>{{ t("adminTeamNamespaces.releaseNamespaceClaim") }}</template>
    <template #description>
      <i18n-t keypath="adminTeamNamespaces.releaseExplanation" tag="span">
        <template #prefix
          ><span class="font-mono">{{ releaseTarget?.prefix }}</span></template
        >
        <template #group
          ><span class="font-mono">{{ releaseTarget?.group_id }}</span></template
        >
      </i18n-t>
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
          {{ t("common.cancel") }}
        </Button>
        <Button variant="destructive" size="sm" :disabled="releaseLoading" @click="confirmRelease">
          {{
            releaseLoading
              ? t("adminTeamNamespaces.releasing")
              : t("adminTeamNamespaces.releaseClaim")
          }}
        </Button>
      </div>
    </div>
  </Dialog>
</template>
