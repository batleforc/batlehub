<script setup lang="ts">
import { listPackages, blockPackage, unblockPackage } from "@/client/sdk.gen";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell,
} from "@/components/ui/table";

// The admin endpoint returns the raw core entity (Vec<PackageSummary>), not PackageSummaryDto.
// Fields differ: package_id is nested; status has blocked_by/blocked_at extras.
interface AdminPackageSummary {
  id: string;
  package_id: {
    registry: string;
    name: string;
    version: string;
    artifact?: string | null;
  };
  status: { status: "available" } | { status: "blocked"; reason: string; blocked_by: string; blocked_at: string };
  last_accessed: string | null;
  access_count: number;
}

const { token } = useAuth();

const { data: packages, error, loading, reload } = useApi<AdminPackageSummary[]>(
  () => listPackages() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

async function block(pkg: AdminPackageSummary) {
  const reason = window.prompt("Block reason:");
  if (!reason) return;
  await blockPackage({
    body: {
      registry: pkg.package_id.registry,
      name: pkg.package_id.name,
      version: pkg.package_id.version,
      artifact: pkg.package_id.artifact,
      reason,
    },
  });
  reload();
}

async function unblock(pkg: AdminPackageSummary) {
  await unblockPackage({
    body: {
      registry: pkg.package_id.registry,
      name: pkg.package_id.name,
      version: pkg.package_id.version,
      artifact: pkg.package_id.artifact,
    },
  });
  reload();
}
</script>

<template>
  <Card>
    <CardHeader class="flex flex-row items-center justify-between space-y-0 pb-4">
      <CardTitle class="text-lg">Admin — Packages</CardTitle>
      <Button variant="outline" size="sm" @click="reload">Refresh</Button>
    </CardHeader>
    <CardContent class="p-0">
      <p v-if="loading" class="p-6 text-sm text-muted-foreground">Loading…</p>
      <p v-else-if="error" class="p-6 text-sm text-destructive">{{ error }}</p>

      <Table v-else-if="packages">
        <TableHeader>
          <TableRow>
            <TableHead>Registry</TableHead>
            <TableHead>Name</TableHead>
            <TableHead>Version</TableHead>
            <TableHead>Artifact</TableHead>
            <TableHead>Status</TableHead>
            <TableHead class="text-right">Downloads</TableHead>
            <TableHead class="text-right">Actions</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableRow
            v-for="(pkg, i) in packages"
            :key="i"
            :class="pkg.status.status === 'blocked' ? 'bg-destructive/5' : ''"
          >
            <TableCell class="font-mono text-xs">{{ pkg.package_id.registry }}</TableCell>
            <TableCell class="font-medium">{{ pkg.package_id.name }}</TableCell>
            <TableCell class="font-mono text-xs">{{ pkg.package_id.version }}</TableCell>
            <TableCell class="text-muted-foreground">{{ pkg.package_id.artifact ?? "—" }}</TableCell>
            <TableCell>
              <Badge :variant="pkg.status.status === 'blocked' ? 'destructive' : 'secondary'">
                {{ pkg.status.status === "blocked" ? `Blocked: ${pkg.status.reason}` : "Available" }}
              </Badge>
            </TableCell>
            <TableCell class="text-right">{{ pkg.access_count }}</TableCell>
            <TableCell class="text-right">
              <Button
                v-if="pkg.status.status === 'blocked'"
                variant="outline"
                size="sm"
                @click="unblock(pkg)"
              >
                Unblock
              </Button>
              <Button
                v-else
                variant="destructive"
                size="sm"
                @click="block(pkg)"
              >
                Block
              </Button>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </CardContent>
  </Card>
</template>
