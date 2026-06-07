<script setup lang="ts">
import { ref, computed, onUnmounted } from "vue";
import { Key, Plus, Trash2, Copy, Check, AlertCircle, Clock } from "@lucide/vue";
import { createToken, listTokens, revokeToken as revokeTokenApi } from "@/client/sdk.gen";
import type { TokenListItem, CreateTokenResponse } from "@/client/types.gen";
import { useApi } from "@/composables/useApi";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import Dialog from "@/components/ui/dialog/Dialog.vue";
import Alert from "@/components/ui/alert/Alert.vue";
import Select from "@/components/ui/select/Select.vue";
import { useAuth } from "@/composables/useAuth";

const { identity } = useAuth();

function apiErrorMessage(err: unknown, fallback: string): string {
  if (err != null && typeof err === "object" && "message" in err) {
    return String((err as Record<string, unknown>)["message"]);
  }
  return fallback;
}

// ── Token list ────────────────────────────────────────────────────────────────

const { data: tokens, loading, error, reload } = useApi<TokenListItem[]>(() => listTokens(), []);

// ── Create dialog ─────────────────────────────────────────────────────────────

const showCreate = ref(false);
const creating = ref(false);
const createError = ref<string | null>(null);

const form = ref({
  name: "",
  expires_in_days: 30,
  role: identity.value?.role === "admin" ? "admin" : "user",
});

const roleOptions = computed(() => {
  const r = identity.value?.role;
  const opts = [{ value: "user", label: "User" }];
  if (r === "admin") opts.push({ value: "admin", label: "Admin" });
  return opts;
});

function openCreate() {
  form.value = {
    name: "",
    expires_in_days: 30,
    role: identity.value?.role === "admin" ? "admin" : "user",
  };
  createError.value = null;
  newToken.value = null;
  showCreate.value = true;
}

async function submitCreate() {
  if (!form.value.name.trim()) {
    createError.value = "Token name is required.";
    return;
  }
  creating.value = true;
  createError.value = null;
  try {
    const { data, error: apiError } = await createToken({
      body: {
        name: form.value.name.trim(),
        expires_in_days: form.value.expires_in_days,
        role: form.value.role,
      },
    });
    if (apiError) {
      createError.value = apiErrorMessage(apiError, "Failed to create token.");
    } else {
      showCreate.value = false;
      newToken.value = (data as CreateTokenResponse | undefined)?.token ?? null;
      newTokenExpiry.value = (data as CreateTokenResponse | undefined)?.expires_at ?? null;
      reload();
      startAutoClear();
    }
  } finally {
    creating.value = false;
  }
}

// ── New token reveal ──────────────────────────────────────────────────────────

const newToken = ref<string | null>(null);
const newTokenExpiry = ref<string | null>(null);
const copied = ref(false);
let autoClearTimer: ReturnType<typeof setTimeout> | null = null;

function startAutoClear() {
  autoClearTimer = setTimeout(() => {
    newToken.value = null;
    newTokenExpiry.value = null;
  }, 60_000);
}

onUnmounted(() => {
  if (autoClearTimer) clearTimeout(autoClearTimer);
});

async function copyToken() {
  if (!newToken.value) return;
  await navigator.clipboard.writeText(newToken.value);
  copied.value = true;
  setTimeout(() => {
    copied.value = false;
  }, 2000);
}

function dismissToken() {
  newToken.value = null;
  newTokenExpiry.value = null;
}

// ── Revoke ────────────────────────────────────────────────────────────────────

const revoking = ref<string | null>(null);
const revokeError = ref<string | null>(null);

async function revokeToken(id: string) {
  revoking.value = id;
  revokeError.value = null;
  try {
    const { error: apiError } = await revokeTokenApi({ path: { id } });
    if (apiError) {
      revokeError.value = apiErrorMessage(apiError, "Failed to revoke token.");
    } else {
      reload();
    }
  } finally {
    revoking.value = null;
  }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function formatDate(iso: string) {
  return new Date(iso).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function daysUntil(iso: string) {
  const diff = new Date(iso).getTime() - Date.now();
  return Math.ceil(diff / 86_400_000);
}

const lifetimePresets = [7, 30, 90];
</script>

<template>
  <div class="space-y-6">
    <!-- Header -->
    <div class="flex items-start justify-between gap-4">
      <div>
        <h1 class="font-mono text-2xl font-bold cyber-text-glow flex items-center gap-2">
          <Key class="h-5 w-5 text-primary" />
          Personal API Tokens
        </h1>
        <p class="mt-1 text-sm text-muted-foreground max-w-xl">
          Create long-lived tokens for programmatic access. Tokens inherit your current role (or
          lower). Maximum lifetime is 90 days. The raw token is shown only once on creation — store
          it securely.
        </p>
      </div>
      <Button class="shrink-0" @click="openCreate">
        <Plus class="h-4 w-4 mr-2" />
        Create token
      </Button>
    </div>

    <!-- New token reveal alert -->
    <Alert v-if="newToken" variant="success" class="relative">
      <Check class="h-4 w-4" />
      <div class="pl-2 space-y-2">
        <p class="font-medium text-sm">Token created — copy it now, it won't be shown again.</p>
        <div class="flex items-center gap-2">
          <code
            class="flex-1 block rounded bg-muted px-3 py-2 text-xs font-mono break-all select-all"
          >
            {{ newToken }}
          </code>
          <Button variant="outline" size="icon" class="shrink-0 h-8 w-8" @click="copyToken">
            <Check v-if="copied" class="h-3.5 w-3.5 text-primary" />
            <Copy v-else class="h-3.5 w-3.5" />
          </Button>
        </div>
        <p v-if="newTokenExpiry" class="text-xs text-muted-foreground">
          Expires: {{ formatDate(newTokenExpiry) }}
        </p>
        <Button variant="ghost" size="sm" class="h-7 text-xs" @click="dismissToken">
          Dismiss (auto-clears in 60 s)
        </Button>
      </div>
    </Alert>

    <!-- Error -->
    <Alert v-if="revokeError" variant="destructive">
      <AlertCircle class="h-4 w-4" />
      <span class="pl-2">{{ revokeError }}</span>
    </Alert>

    <!-- Token table -->
    <Card>
      <CardHeader class="pb-3">
        <CardTitle class="text-base"> Active Tokens </CardTitle>
        <CardDescription>
          Tokens that have not been revoked and have not yet expired.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div v-if="loading" class="py-8 text-center text-sm text-muted-foreground">Loading…</div>
        <div v-else-if="error" class="py-8 text-center text-sm text-destructive">
          {{ error }}
        </div>
        <div v-else-if="!tokens?.length" class="py-12 text-center space-y-2">
          <Key class="h-8 w-8 mx-auto text-muted-foreground/50" />
          <p class="text-sm text-muted-foreground">No active tokens. Create one to get started.</p>
        </div>
        <Table v-else>
          <TableHeader>
            <TableRow>
              <TableHead>Name</TableHead>
              <TableHead>Role</TableHead>
              <TableHead>Expires</TableHead>
              <TableHead>Created</TableHead>
              <TableHead class="w-16" />
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow v-for="tok in tokens" :key="tok.id">
              <TableCell class="font-medium">
                {{ tok.name }}
              </TableCell>
              <TableCell>
                <Badge :variant="tok.role === 'admin' ? 'default' : 'secondary'" class="text-xs">
                  {{ tok.role }}
                </Badge>
              </TableCell>
              <TableCell>
                <span
                  :class="
                    daysUntil(tok.expires_at) <= 7
                      ? 'text-destructive font-medium'
                      : 'text-muted-foreground'
                  "
                  class="text-sm flex items-center gap-1"
                >
                  <Clock v-if="daysUntil(tok.expires_at) <= 7" class="h-3 w-3" />
                  {{ formatDate(tok.expires_at) }}
                  <span class="text-xs opacity-70">({{ daysUntil(tok.expires_at) }}d)</span>
                </span>
              </TableCell>
              <TableCell class="text-sm text-muted-foreground">
                {{ formatDate(tok.created_at) }}
              </TableCell>
              <TableCell>
                <Button
                  variant="ghost"
                  size="icon"
                  class="h-7 w-7 text-muted-foreground hover:text-destructive"
                  :disabled="revoking === tok.id"
                  title="Revoke token"
                  @click="revokeToken(tok.id)"
                >
                  <Trash2 class="h-3.5 w-3.5" />
                </Button>
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
      </CardContent>
    </Card>

    <!-- Create dialog -->
    <Dialog :open="showCreate" @update:open="showCreate = $event">
      <template #default>
        <div class="space-y-1 pr-6">
          <h2 class="text-lg font-semibold">Create API Token</h2>
          <p class="text-sm text-muted-foreground">
            Choose a name, role, and lifetime for your token.
          </p>
        </div>

        <div class="space-y-4 mt-2">
          <div class="space-y-1.5">
            <Label for="token-name">Name</Label>
            <Input
              id="token-name"
              v-model="form.name"
              placeholder="e.g. CI pipeline"
              @keyup.enter="submitCreate"
            />
          </div>

          <div class="space-y-1.5">
            <Label>Role</Label>
            <Select v-model="form.role" :options="roleOptions" placeholder="Select role" />
          </div>

          <div class="space-y-2">
            <Label>Lifetime</Label>
            <div class="flex gap-2">
              <Button
                v-for="days in lifetimePresets"
                :key="days"
                :variant="form.expires_in_days === days ? 'default' : 'outline'"
                size="sm"
                @click="form.expires_in_days = days"
              >
                {{ days }}d
              </Button>
            </div>
            <div class="flex items-center gap-2 text-sm text-muted-foreground">
              <span>or custom:</span>
              <Input
                type="number"
                min="1"
                max="90"
                :value="form.expires_in_days"
                class="w-24 h-8"
                @input="
                  form.expires_in_days = Math.min(
                    90,
                    Math.max(1, +($event.target as HTMLInputElement).value),
                  )
                "
              />
              <span>days</span>
            </div>
          </div>

          <Alert v-if="createError" variant="destructive" class="text-sm py-2">
            <AlertCircle class="h-3.5 w-3.5" />
            <span class="pl-2">{{ createError }}</span>
          </Alert>

          <div class="flex justify-end gap-2 pt-2">
            <Button variant="outline" :disabled="creating" @click="showCreate = false">
              Cancel
            </Button>
            <Button :disabled="creating" @click="submitCreate">
              {{ creating ? "Creating…" : "Create token" }}
            </Button>
          </div>
        </div>
      </template>
    </Dialog>
  </div>
</template>
