<script setup lang="ts">
import { listPackages2 } from "@/client/sdk.gen";
import type { PackageListResponse, PackageSummaryDto } from "@/client/types.gen";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell,
} from "@/components/ui/table";

const { token } = useAuth();

// listPackages2 returns PackageListResponse { items, total, page, per_page }
const { data, error, loading, reload } = useApi<PackageListResponse>(
  () => listPackages2() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

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
    <CardHeader class="flex flex-row items-center justify-between space-y-0 pb-4">
      <CardTitle class="text-lg">
        Packages
        <span v-if="data" class="font-normal text-muted-foreground text-base ml-1">
          ({{ data.total }})
        </span>
      </CardTitle>
      <Button variant="outline" size="sm" @click="reload">Refresh</Button>
    </CardHeader>
    <CardContent class="p-0">
      <p v-if="loading" class="p-6 text-sm text-muted-foreground">Loading…</p>
      <p v-else-if="error" class="p-6 text-sm text-destructive">{{ error }}</p>

      <Table v-else-if="data">
        <TableHeader>
          <TableRow>
            <TableHead>Registry</TableHead>
            <TableHead>Name</TableHead>
            <TableHead>Version</TableHead>
            <TableHead>Artifact</TableHead>
            <TableHead>Status</TableHead>
            <TableHead class="text-right">Downloads</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableRow v-for="(pkg, i) in data.items" :key="i">
            <TableCell class="font-mono text-xs">{{ pkg.registry }}</TableCell>
            <TableCell class="font-medium">{{ pkg.name }}</TableCell>
            <TableCell class="font-mono text-xs">{{ pkg.version }}</TableCell>
            <TableCell class="text-muted-foreground">{{ pkg.artifact ?? "—" }}</TableCell>
            <TableCell>
              <Badge :variant="statusVariant(pkg)">{{ statusLabel(pkg) }}</Badge>
            </TableCell>
            <TableCell class="text-right">{{ pkg.access_count }}</TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </CardContent>
  </Card>
</template>
