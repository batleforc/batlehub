<script setup lang="ts">
import { ref } from "vue";
import { checkAccess } from "@/client/sdk.gen";
import type { AccessCheckResponse } from "@/client/types.gen";
import { API_BASE_URL } from "@/config";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";

const registry = ref("github");
const name = ref("");
const version = ref("");
const artifact = ref("");
const result = ref<AccessCheckResponse | null>(null);
const error = ref<string | null>(null);
const loading = ref(false);

async function check() {
  loading.value = true;
  error.value = null;
  result.value = null;
  try {
    const { data, error: apiErr } = await checkAccess({
      query: {
        registry: registry.value,
        name: name.value,
        version: version.value,
        artifact: artifact.value || null,
      },
    });
    if (apiErr || !data) {
      error.value = apiErr ? String(apiErr) : "No response";
    } else {
      result.value = data;
    }
  } catch (e) {
    error.value = String(e);
  } finally {
    loading.value = false;
  }
}
</script>

<template>
  <Card class="max-w-lg">
    <CardHeader>
      <CardTitle class="text-lg"> Access Check </CardTitle>
    </CardHeader>
    <CardContent class="space-y-4">
      <div class="grid gap-3">
        <div class="space-y-1">
          <Label for="registry">Registry</Label>
          <Input id="registry" v-model="registry" placeholder="github" />
        </div>
        <div class="space-y-1">
          <Label for="name">Name (owner/repo)</Label>
          <Input id="name" v-model="name" placeholder="owner/repo" />
        </div>
        <div class="space-y-1">
          <Label for="version">Version</Label>
          <Input id="version" v-model="version" placeholder="v1.0.0" />
        </div>
        <div class="space-y-1">
          <Label for="artifact">Artifact (optional)</Label>
          <Input id="artifact" v-model="artifact" placeholder="12345678" />
        </div>
      </div>

      <Button :disabled="loading" class="w-full" @click="check">
        {{ loading ? "Checking…" : "Check Access" }}
      </Button>

      <p v-if="error" class="text-sm text-destructive">
        {{ error }}
      </p>

      <div v-if="result" class="rounded-sm border p-4 space-y-2">
        <div class="flex items-center gap-2">
          <Badge :variant="result.can_access ? 'default' : 'destructive'">
            {{ result.can_access ? "Allowed" : "Denied" }}
          </Badge>
          <span v-if="!result.can_access" class="text-sm text-muted-foreground">
            {{ result.reason ?? "no reason given" }}
          </span>
        </div>
        <p v-if="result.proxy_url" class="text-xs text-muted-foreground break-all">
          URL:
          <a
            :href="`${API_BASE_URL}${result.proxy_url}`"
            target="_blank"
            rel="noopener"
            class="font-mono underline underline-offset-2 hover:text-foreground"
            >{{ API_BASE_URL }}{{ result.proxy_url }}</a
          >
        </p>
      </div>
    </CardContent>
  </Card>
</template>
