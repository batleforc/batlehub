<script setup lang="ts">
import { useI18n } from "vue-i18n";
/**
 * The one route that adapts to who is asking (RFC 0003 §4.3).
 *
 * `/` used to redirect blindly to `/packages`, which meant a freshly deployed
 * instance greeted its operator with an empty table and no next action. The
 * three audiences arriving here want different things, and the instance's own
 * state decides which of them is being served:
 *
 *   - fresh instance + admin  → what is configured, what is missing, what to do
 *   - anyone else             → straight to work, no ceremony
 *
 * The probe is one call the catalog already makes. It never *writes* config —
 * first-run guidance explains and links; it does not gain a privileged endpoint.
 */
import { computed } from "vue";
import { RouterLink } from "vue-router";
import { listRegistries } from "@/client/sdk.gen";
import type { RegistryInfo } from "@/client/types.gen";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { DOCS_URL } from "@/config";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/ui/empty-state";
import { Skeleton } from "@/components/ui/skeleton";
import { Alert } from "@/components/ui/alert";

const { t } = useI18n();

const { identity, isAdmin, isAuthenticated, token } = useAuth();

const { data, error, loading } = useApi<RegistryInfo[]>(
  () => listRegistries() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

const registries = computed<RegistryInfo[]>(() => data.value ?? []);
const isFresh = computed(() => !loading.value && !error.value && registries.value.length === 0);
const publishable = computed(() => registries.value.filter((r) => r.mode !== "proxy").length);
const name = computed(() => identity.value?.user_id ?? "");
</script>

<template>
  <div class="space-y-6">
    <header class="space-y-1">
      <!-- Pixel Medium: DESIGN.md reserves this step for the wordmark, and the
           home route is the one view whose title *is* the wordmark. -->
      <h1 class="font-display text-2xl font-bold tracking-[0.04em]">
        BatleHub<span class="text-primary">.</span>
      </h1>
      <p class="text-sm text-muted-foreground">
        <template v-if="isAuthenticated">{{ t("home.signedInAs", { user: name }) }}</template>
        <template v-else>{{ t("homePage.aCacheAndRegistry") }}</template>
      </p>
    </header>

    <Skeleton v-if="loading" :lines="4" />

    <Alert v-else-if="error" variant="destructive">{{ error }}</Alert>

    <!-- Fresh instance. The operator is the only one who can act on this, so
         only they are told how; everyone else is told what it means for them. -->
    <EmptyState
      v-else-if="isFresh"
      :title="isAdmin ? t('adminDashboard.noRegistriesConfiguredYet') : t('home.freshTitleOther')"
      :description="isAdmin ? t('home.freshBodyAdmin') : t('home.freshBodyOther')"
    >
      <template v-if="isAdmin" #action>
        <Button as-child size="sm">
          <RouterLink to="/admin/operations/config-reload">{{
            t("homePage.openConfig")
          }}</RouterLink>
        </Button>
        <Button as-child size="sm" variant="outline">
          <a :href="DOCS_URL" target="_blank" rel="noopener noreferrer">{{
            t("homePage.configurationGuide")
          }}</a>
        </Button>
      </template>
    </EmptyState>

    <!-- Working instance: the shortest path to the two things people come for. -->
    <template v-else>
      <dl class="grid gap-px border border-border bg-border sm:grid-cols-3">
        <div class="bg-background p-4">
          <dt class="font-mono text-xs uppercase tracking-wider text-muted-foreground">
            {{ t("common.registries") }}
          </dt>
          <dd class="mt-1 font-mono text-2xl tabular-nums">{{ registries.length }}</dd>
        </div>
        <div class="bg-background p-4">
          <dt class="font-mono text-xs uppercase tracking-wider text-muted-foreground">
            {{ t("homePage.acceptingPublishes") }}
          </dt>
          <dd class="mt-1 font-mono text-2xl tabular-nums">{{ publishable }}</dd>
        </div>
        <div class="bg-background p-4">
          <dt class="font-mono text-xs uppercase tracking-wider text-muted-foreground">
            {{ t("common.you") }}
          </dt>
          <dd class="mt-1 font-mono text-2xl">{{ identity?.role ?? "anonymous" }}</dd>
        </div>
      </dl>

      <div class="flex flex-wrap gap-2">
        <Button as-child size="sm">
          <RouterLink to="/explore">{{ t("homePage.browsePackages") }}</RouterLink>
        </Button>
        <Button as-child size="sm" variant="outline">
          <RouterLink to="/setup">{{ t("homePage.pointAToolAt") }}</RouterLink>
        </Button>
        <Button v-if="isAuthenticated" as-child size="sm" variant="outline">
          <RouterLink to="/me/namespace">{{ t("homePage.myNamespace") }}</RouterLink>
        </Button>
        <Button v-if="isAdmin" as-child size="sm" variant="outline">
          <RouterLink to="/admin/dashboard">{{ t("common.admin") }}</RouterLink>
        </Button>
      </div>
    </template>
  </div>
</template>
