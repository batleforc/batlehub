<script setup lang="ts">
import { ref, computed } from "vue";
import { useRouter } from "vue-router";
import { Package } from "@lucide/vue";
import { listPackages2 } from "@/client/sdk.gen";
import type { PackageListResponse, PackageSummaryDto } from "@/client/types.gen";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell,
} from "@/components/ui/table";

const { token } = useAuth();
const router = useRouter();

const { data, error, loading, reload } = useApi<PackageListResponse>(
  () => listPackages2() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

const search = ref("");

const filteredItems = computed(() => {
  if (!data.value?.items) return [];
  const q = search.value.toLowerCase().trim();
  if (!q) return data.value.items;
  return data.value.items.filter((p) =>
    p.name.toLowerCase().includes(q) ||
    p.registry.toLowerCase().includes(q) ||
    p.version.toLowerCase().includes(q),
  );
});

function statusVariant(pkg: PackageSummaryDto) {
  return pkg.status.status === "blocked" ? "destructive" : "secondary";
}

function statusLabel(pkg: PackageSummaryDto) {
  return pkg.status.status === "blocked"
    ? `Blocked: ${pkg.status.reason}`
    : "Available";
}
</script>

<template>
  <Card>
    <CardHeader class="space-y-3 pb-3">
      <div class="flex flex-row items-center justify-between space-y-0">
        <CardTitle class="text-lg">
          Packages
          <span
            v-if="data"
            class="font-normal text-muted-foreground text-base ml-1"
          >
            ({{ data.total }})
          </span>
        </CardTitle>
        <Button
          variant="outline"
          size="sm"
          @click="reload"
        >
          Refresh
        </Button>
      </div>
      <Input
        v-model="search"
        placeholder="Filter by name, registry, or version…"
        class="max-w-sm h-8 text-sm"
      />
    </CardHeader>
    <CardContent class="p-0">
      <p
        v-if="loading"
        class="p-6 text-sm text-muted-foreground"
      >
        Loading…
      </p>
      <p
        v-else-if="error"
        class="p-6 text-sm text-destructive"
      >
        {{ error }}
      </p>

      <Table v-else-if="filteredItems.length">
        <TableHeader>
          <TableRow>
            <TableHead>Registry</TableHead>
            <TableHead>Name</TableHead>
            <TableHead>Version</TableHead>
            <TableHead>Artifact</TableHead>
            <TableHead>Status</TableHead>
            <TableHead class="text-right">
              Downloads
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableRow
            v-for="(pkg, i) in filteredItems"
            :key="i"
            class="cursor-pointer hover:bg-muted/50"
            @click="router.push({ path: '/packages/detail', query: { registry: pkg.registry, name: pkg.name, version: pkg.version, ...(pkg.artifact ? { artifact: pkg.artifact } : {}) } })"
          >
            <TableCell class="font-mono text-xs">
              {{ pkg.registry }}
            </TableCell>
            <TableCell class="font-medium">
              {{ pkg.name }}
            </TableCell>
            <TableCell class="font-mono text-xs">
              {{ pkg.version }}
            </TableCell>
            <TableCell class="text-muted-foreground font-mono text-xs">
              {{ pkg.artifact ?? "—" }}
            </TableCell>
            <TableCell>
              <Badge :variant="statusVariant(pkg)">
                {{ statusLabel(pkg) }}
              </Badge>
            </TableCell>
            <TableCell class="text-right tabular-nums">
              {{ pkg.access_count }}
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>

      <div
        v-else-if="!loading"
        class="py-12 text-center space-y-2"
      >
        <Package class="h-8 w-8 mx-auto text-muted-foreground/50" />
        <p class="text-sm text-muted-foreground">
          {{ search ? "No packages match your filter." : "No packages cached yet." }}
        </p>
        <p
          v-if="!search"
          class="text-xs text-muted-foreground"
        >
          Packages appear here once they are downloaded through the proxy.
        </p>
      </div>
    </CardContent>
  </Card>
</template>
