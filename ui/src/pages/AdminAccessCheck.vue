<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { onMounted, ref } from "vue";
import { useRoute, RouterLink } from "vue-router";
import { adminAccessCheck } from "@/client/sdk.gen";
import { extractMessage } from "@/composables/useApi";
import SectionTabs from "@/components/admin/SectionTabs.vue";
import { SECURITY_TABS } from "@/config/adminSections";
import { PageHeader } from "@/components/ui/page-header";

const { t } = useI18n();
const route = useRoute();

const registry = ref("");
const packageName = ref("");
const version = ref("1.0.0");
const resourceType = ref("releases:read");
const userId = ref("");
const role = ref("anonymous");
const groups = ref("");

const result = ref<null | { decision: string; reason?: string; rule_matched?: string }>(null);
const loading = ref(false);
const error = ref<string | null>(null);

const RESOURCE_TYPES = ["releases:read", "source:read", "releases:write", "source:write"];

/**
 * Prefill from the query, as `/tools/access-check` already does.
 *
 * "Nobody opens an access checker for fun; they open it because something was
 * refused" — the project's own tested premise for the public checker. This page
 * had seven empty fields and read no query at all, so an operator who had just
 * seen a denial in the audit log retyped every coordinate by hand.
 */
onMounted(() => {
  const q = route.query;
  const one = (v: unknown) => (typeof v === "string" ? v : "");
  registry.value = one(q.registry) || registry.value;
  packageName.value = one(q.name) || packageName.value;
  version.value = one(q.version) || version.value;
  userId.value = one(q.user_id) || userId.value;
  role.value = one(q.role) || role.value;
});

async function simulate() {
  loading.value = true;
  error.value = null;
  result.value = null;
  try {
    const body: Record<string, unknown> = {
      registry: registry.value,
      package_name: packageName.value,
      version: version.value,
      resource_type: resourceType.value,
      role: role.value || "anonymous",
    };
    if (userId.value) body.user_id = userId.value;
    const grps = groups.value
      .split(",")
      .map((g) => g.trim())
      .filter(Boolean);
    if (grps.length) body.groups = grps;

    // The generated client, not a bare relative `fetch`. There is no `/api`
    // proxy in front of the SPA (CLAUDE.md), so `fetch("/api/v1/...")` POSTed
    // to the Vite origin on both dev servers and on every deployment where the
    // API is not same-origin. `adminAccessCheck` was already generated and
    // typed, and carries the base URL and the auth header for free.
    const res = await adminAccessCheck({ body: body as never });
    if (res.error) {
      error.value = extractMessage(res.error);
      return;
    }
    result.value = (res.data ?? null) as typeof result.value;
  } catch (e: unknown) {
    error.value = e instanceof Error ? e.message : String(e);
  } finally {
    loading.value = false;
  }
}
</script>

<template>
  <div class="space-y-6">
    <SectionTabs :tabs="SECURITY_TABS" />
    <PageHeader
      variant="display"
      :title="t('adminAccessCheck.rbacAccessCheck')"
      :description="t('adminAccessCheck.simulateWhetherAnIdentityWould')"
    />

    <form @submit.prevent="simulate" class="space-y-4 max-w-lg">
      <div class="grid grid-cols-2 gap-4">
        <div class="space-y-1">
          <label for="aac-registry" class="text-sm font-medium">{{ t("common.registry") }}</label>
          <input
            id="aac-registry"
            v-model="registry"
            required
            placeholder="npm"
            class="w-full rounded border border-border bg-background px-3 py-1.5 text-sm"
          />
        </div>
        <div class="space-y-1">
          <label for="aac-package" class="text-sm font-medium">{{
            t("adminAccessCheck.packageName")
          }}</label>
          <input
            id="aac-package"
            v-model="packageName"
            required
            placeholder="lodash"
            class="w-full rounded border border-border bg-background px-3 py-1.5 text-sm"
          />
        </div>
        <div class="space-y-1">
          <label for="aac-version" class="text-sm font-medium">{{ t("common.version") }}</label>
          <input
            id="aac-version"
            v-model="version"
            required
            placeholder="1.0.0"
            class="w-full rounded border border-border bg-background px-3 py-1.5 text-sm"
          />
        </div>
        <div class="space-y-1">
          <label for="aac-resource-type" class="text-sm font-medium">{{
            t("adminAccessCheck.resourceType")
          }}</label>
          <select
            id="aac-resource-type"
            v-model="resourceType"
            class="w-full rounded border border-border bg-background px-3 py-1.5 text-sm"
          >
            <option v-for="rt in RESOURCE_TYPES" :key="rt" :value="rt">{{ rt }}</option>
          </select>
        </div>
      </div>

      <hr class="border-border" />

      <p class="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
        {{ t("adminAccessCheck.simulatedIdentity") }}
      </p>

      <div class="grid grid-cols-2 gap-4">
        <div class="space-y-1">
          <label for="aac-role" class="text-sm font-medium">{{ t("common.role") }}</label>
          <select
            id="aac-role"
            v-model="role"
            class="w-full rounded border border-border bg-background px-3 py-1.5 text-sm"
          >
            <option value="anonymous">anonymous</option>
            <option value="user">user</option>
            <option value="admin">admin</option>
          </select>
        </div>
        <div class="space-y-1">
          <label for="aac-user-id" class="text-sm font-medium"
            >{{ t("adminAccessCheck.userId") }}
            <span class="text-muted-foreground">{{ t("adminAccessCheck.optional") }}</span></label
          >
          <input
            id="aac-user-id"
            v-model="userId"
            placeholder="alice"
            class="w-full rounded border border-border bg-background px-3 py-1.5 text-sm"
          />
        </div>
        <div class="col-span-2 space-y-1">
          <label for="aac-groups" class="text-sm font-medium"
            >{{ t("common.groups") }}
            <span class="text-muted-foreground">{{
              t("adminAccessCheck.commaSeparated")
            }}</span></label
          >
          <input
            id="aac-groups"
            v-model="groups"
            :placeholder="t('adminAccessCheck.oidc1TeamATeam')"
            class="w-full rounded border border-border bg-background px-3 py-1.5 text-sm"
          />
        </div>
      </div>

      <button
        type="submit"
        :disabled="loading"
        class="rounded bg-primary text-primary-foreground px-4 py-1.5 text-sm font-medium disabled:opacity-50"
      >
        {{ loading ? t("accessCheck.checking") : t("adminAccessCheck.checkAccess") }}
      </button>
    </form>

    <div
      v-if="error"
      class="rounded border border-destructive/50 px-4 py-3 text-sm text-destructive"
    >
      {{ error }}
    </div>

    <div
      v-if="result"
      class="rounded border px-4 py-3 space-y-1"
      :class="result.decision === 'allow' ? 'border-foreground/50' : 'border-destructive/50'"
    >
      <p
        class="font-semibold text-sm"
        :class="result.decision === 'allow' ? 'text-foreground' : 'text-destructive'"
      >
        {{ result.decision === "allow" ? t("adminAccessCheck.allow") : t("adminAccessCheck.deny") }}
      </p>
      <p v-if="result.reason" class="text-sm text-muted-foreground">{{ result.reason }}</p>
      <p v-if="result.rule_matched" class="text-xs text-muted-foreground">
        {{ t("adminAccessCheck.ruleMatched", { rule: result.rule_matched }) }}
      </p>
      <!--
        State the bound. The handler evaluates registry policy *rules* only —
        it never consults the user-block or IP-block stores, and both of those
        reject in middleware *before* the rules run. So this page can answer
        "allow" about an account the next tab shows as blocked. Until the
        endpoint learns about them, the honest thing is to say what the
        simulation covers rather than let silence imply completeness.
      -->
      <p class="border-t border-border pt-2 text-xs text-muted-foreground">
        {{ t("adminAccessCheck.simulationBound") }}
        <RouterLink to="/admin/security/blocks" class="text-primary hover:underline">{{
          t("adminAccessCheck.seeBlocks")
        }}</RouterLink>
      </p>
    </div>
  </div>
</template>
