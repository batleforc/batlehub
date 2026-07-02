<script setup lang="ts">
import { ref } from "vue";
import { useAuth } from "@/composables/useAuth";
import SectionTabs from "@/components/admin/SectionTabs.vue";
import { SECURITY_TABS } from "@/config/adminSections";
import { PageHeader } from "@/components/ui/page-header";

const { token } = useAuth();

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

async function simulate() {
  loading.value = true;
  error.value = null;
  result.value = null;
  try {
    const headers: Record<string, string> = { "Content-Type": "application/json" };
    if (token.value) headers["Authorization"] = `Bearer ${token.value}`;

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

    const resp = await fetch("/api/v1/admin/access-check", {
      method: "POST",
      headers,
      body: JSON.stringify(body),
    });
    if (!resp.ok) throw new Error(`HTTP ${resp.status}: ${await resp.text()}`);
    result.value = await resp.json();
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
      title="RBAC Access Check"
      description="Simulate whether an identity would be allowed to access a package resource under the current registry policy — without making a real request."
    />

    <form @submit.prevent="simulate" class="space-y-4 max-w-lg">
      <div class="grid grid-cols-2 gap-4">
        <div class="space-y-1">
          <label for="aac-registry" class="text-sm font-medium">Registry</label>
          <input
            id="aac-registry"
            v-model="registry"
            required
            placeholder="npm"
            class="w-full rounded border border-border bg-background px-3 py-1.5 text-sm"
          />
        </div>
        <div class="space-y-1">
          <label for="aac-package" class="text-sm font-medium">Package name</label>
          <input
            id="aac-package"
            v-model="packageName"
            required
            placeholder="lodash"
            class="w-full rounded border border-border bg-background px-3 py-1.5 text-sm"
          />
        </div>
        <div class="space-y-1">
          <label for="aac-version" class="text-sm font-medium">Version</label>
          <input
            id="aac-version"
            v-model="version"
            required
            placeholder="1.0.0"
            class="w-full rounded border border-border bg-background px-3 py-1.5 text-sm"
          />
        </div>
        <div class="space-y-1">
          <label for="aac-resource-type" class="text-sm font-medium">Resource type</label>
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
        Simulated identity
      </p>

      <div class="grid grid-cols-2 gap-4">
        <div class="space-y-1">
          <label for="aac-role" class="text-sm font-medium">Role</label>
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
            >User ID <span class="text-muted-foreground">(optional)</span></label
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
            >Groups <span class="text-muted-foreground">(comma-separated)</span></label
          >
          <input
            id="aac-groups"
            v-model="groups"
            placeholder="oidc1:team-a, team-b"
            class="w-full rounded border border-border bg-background px-3 py-1.5 text-sm"
          />
        </div>
      </div>

      <button
        type="submit"
        :disabled="loading"
        class="rounded bg-primary text-primary-foreground px-4 py-1.5 text-sm font-medium disabled:opacity-50"
      >
        {{ loading ? "Checking…" : "Check access" }}
      </button>
    </form>

    <div
      v-if="error"
      class="rounded border border-destructive/50 bg-destructive/10 px-4 py-3 text-sm text-destructive"
    >
      {{ error }}
    </div>

    <div
      v-if="result"
      class="rounded border px-4 py-3 space-y-1"
      :class="
        result.decision === 'allow'
          ? 'border-green-500/50 bg-green-500/10'
          : 'border-red-500/50 bg-red-500/10'
      "
    >
      <p
        class="font-semibold text-sm"
        :class="
          result.decision === 'allow'
            ? 'text-green-700 dark:text-green-400'
            : 'text-red-700 dark:text-red-400'
        "
      >
        {{ result.decision === "allow" ? "✓ ALLOW" : "✗ DENY" }}
      </p>
      <p v-if="result.reason" class="text-sm text-muted-foreground">{{ result.reason }}</p>
      <p v-if="result.rule_matched" class="text-xs text-muted-foreground">
        Rule: {{ result.rule_matched }}
      </p>
    </div>
  </div>
</template>
