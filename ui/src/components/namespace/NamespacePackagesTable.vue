<script setup lang="ts">
import { ref, watch, toRef } from "vue";
import { myNamespacePackages, setPackageVisibility } from "@/client/sdk.gen";
import type {
  Visibility,
  TeamNamespaceDto,
  NamespacePackageDto,
} from "@/lib/registry-types";
import { VISIBILITY_OPTIONS } from "@/lib/registry-types";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import Select from "@/components/ui/select/Select.vue";

const props = defineProps<{ namespace: TeamNamespaceDto }>();

const { token } = useAuth();
const page = ref(0);
const PAGE_SIZE = 50;

const nsRef = toRef(props, "namespace");

const {
  data: pkgsData,
  error,
  loading,
} = useApi<NamespacePackageDto[]>(
  () =>
    myNamespacePackages({
      path: {
        registry: props.namespace.registry,
        prefix: props.namespace.prefix,
      },
      query: { page: page.value, per_page: PAGE_SIZE },
    }) as Promise<{ data?: unknown; error?: unknown }>,
  [token, nsRef, page],
);

watch(nsRef, () => {
  page.value = 0;
});

function prevPage() {
  if (page.value > 0) page.value--;
}
function nextPage() {
  if ((pkgsData.value?.length ?? 0) >= PAGE_SIZE) page.value++;
}

// ── Inline visibility editing ─────────────────────────────────────────────────

const editing = ref<Record<string, Visibility>>({});
const saving = ref<Record<string, boolean>>({});
const saveError = ref<Record<string, string>>({});

const visibilityOptions = VISIBILITY_OPTIONS.map((o) => ({
  value: o.value,
  label: o.label.split(" —")[0],
}));

function pkgKey(pkg: NamespacePackageDto) {
  return `${props.namespace.registry}|${pkg.name}|${pkg.version}`;
}

function startEdit(pkg: NamespacePackageDto) {
  editing.value = { ...editing.value, [pkgKey(pkg)]: pkg.visibility };
}

function cancelEdit(pkg: NamespacePackageDto) {
  const copy = { ...editing.value };
  delete copy[pkgKey(pkg)];
  editing.value = copy;
}

async function saveVis(pkg: NamespacePackageDto) {
  const k = pkgKey(pkg);
  const vis = editing.value[k];
  saving.value = { ...saving.value, [k]: true };
  saveError.value = { ...saveError.value, [k]: "" };
  try {
    const { error: apiErr } = await setPackageVisibility({
      path: { registry: props.namespace.registry, name: pkg.name },
      body: { visibility: vis },
    });
    if (apiErr)
      throw new Error((apiErr as { message?: string })?.message ?? "API error");
    pkg.visibility = vis;
    cancelEdit(pkg);
  } catch (e) {
    saveError.value = {
      ...saveError.value,
      [k]: e instanceof Error ? e.message : "Unknown error",
    };
  } finally {
    saving.value = { ...saving.value, [k]: false };
  }
}

function visVariant(v: Visibility): "default" | "secondary" | "outline" {
  if (v === "public") return "default";
  return v === "internal" ? "secondary" : "outline";
}

function formatDate(iso: string) {
  return new Date(iso).toLocaleDateString(undefined, { dateStyle: "medium" });
}
</script>

<template>
  <p v-if="loading" class="text-sm text-muted-foreground">Loading…</p>
  <p v-else-if="error" class="text-sm text-destructive">{{ error }}</p>
  <p v-else-if="!pkgsData?.length" class="text-sm text-muted-foreground">
    No published packages found under this namespace.
  </p>
  <template v-else>
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Package</TableHead>
          <TableHead>Version</TableHead>
          <TableHead>Visibility</TableHead>
          <TableHead>Published by</TableHead>
          <TableHead>Date</TableHead>
          <TableHead />
        </TableRow>
      </TableHeader>
      <TableBody>
        <TableRow
          v-for="pkg in pkgsData"
          :key="`${pkg.name}@${pkg.version}`"
          :class="pkg.yanked ? 'opacity-50' : ''"
        >
          <TableCell class="font-mono text-xs">{{ pkg.name }}</TableCell>
          <TableCell class="font-mono text-xs">
            {{ pkg.version }}
            <span v-if="pkg.yanked" class="ml-1 text-destructive"
              >(yanked)</span
            >
          </TableCell>
          <TableCell>
            <template v-if="editing[pkgKey(pkg)] !== undefined">
              <div class="flex items-center gap-1">
                <Select
                  v-model="editing[pkgKey(pkg)]"
                  :options="visibilityOptions"
                  class="w-32 text-xs"
                />
                <Button
                  size="sm"
                  variant="default"
                  :disabled="saving[pkgKey(pkg)]"
                  class="text-xs h-7 px-2"
                  @click="saveVis(pkg)"
                >
                  {{ saving[pkgKey(pkg)] ? "…" : "Save" }}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  class="text-xs h-7 px-2"
                  @click="cancelEdit(pkg)"
                  >Cancel</Button
                >
              </div>
              <p
                v-if="saveError[pkgKey(pkg)]"
                class="text-xs text-destructive mt-0.5"
              >
                {{ saveError[pkgKey(pkg)] }}
              </p>
            </template>
            <Badge
              v-else
              :variant="visVariant(pkg.visibility)"
              class="capitalize text-xs cursor-pointer"
              :class="
                pkg.visibility === 'team' ? 'border-primary text-primary' : ''
              "
              @click="startEdit(pkg)"
            >
              {{ pkg.visibility }}
            </Badge>
          </TableCell>
          <TableCell class="text-xs">{{ pkg.published_by }}</TableCell>
          <TableCell class="text-xs">{{
            formatDate(pkg.published_at)
          }}</TableCell>
          <TableCell>
            <Button
              v-if="editing[pkgKey(pkg)] === undefined"
              size="sm"
              variant="ghost"
              class="text-xs h-7 px-2"
              @click="startEdit(pkg)"
            >
              Edit visibility
            </Button>
          </TableCell>
        </TableRow>
      </TableBody>
    </Table>
    <div class="flex items-center justify-between mt-3">
      <Button
        variant="outline"
        size="sm"
        :disabled="page === 0"
        @click="prevPage"
        >Previous</Button
      >
      <span class="text-xs text-muted-foreground">Page {{ page + 1 }}</span>
      <Button
        variant="outline"
        size="sm"
        :disabled="(pkgsData?.length ?? 0) < PAGE_SIZE"
        @click="nextPage"
        >Next</Button
      >
    </div>
  </template>
</template>
