<script setup lang="ts">
import { auditLog } from "@/client/sdk.gen";
import type { PackageIdentifierDto } from "@/client/types.gen";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell,
} from "@/components/ui/table";

interface AccessEvent {
  id: string;
  package_id: PackageIdentifierDto;
  user_id?: string;
  action: string;
  result: { result: "allowed" } | { result: "denied"; reason: string };
  occurred_at: string;
}

interface AuditResponse {
  items: AccessEvent[];
  total: number;
  page: number;
  per_page: number;
}

const { token } = useAuth();

const { data, error, loading, reload } = useApi<AuditResponse>(
  () => auditLog() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);
</script>

<template>
  <Card>
    <CardHeader class="flex flex-row items-center justify-between space-y-0 pb-4">
      <CardTitle class="text-lg">
        Admin — Audit Log
        <span v-if="data" class="font-normal text-muted-foreground text-base">
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
            <TableHead>Time</TableHead>
            <TableHead>User</TableHead>
            <TableHead>Registry</TableHead>
            <TableHead>Package</TableHead>
            <TableHead>Action</TableHead>
            <TableHead>Result</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableRow
            v-for="ev in data.items"
            :key="ev.id"
            :class="ev.result.result === 'denied' ? 'bg-destructive/5' : ''"
          >
            <TableCell class="whitespace-nowrap text-xs">
              {{ new Date(ev.occurred_at).toLocaleString() }}
            </TableCell>
            <TableCell class="text-sm">
              <span v-if="ev.user_id">{{ ev.user_id }}</span>
              <span v-else class="text-muted-foreground italic">anonymous</span>
            </TableCell>
            <TableCell class="font-mono text-xs">{{ ev.package_id.registry }}</TableCell>
            <TableCell class="font-mono text-xs">
              {{ ev.package_id.name }}@{{ ev.package_id.version }}
              <span v-if="ev.package_id.artifact" class="text-muted-foreground">
                ({{ ev.package_id.artifact }})
              </span>
            </TableCell>
            <TableCell class="text-xs">{{ ev.action }}</TableCell>
            <TableCell>
              <Badge :variant="ev.result.result === 'denied' ? 'destructive' : 'secondary'">
                {{ ev.result.result === "denied"
                  ? `Denied: ${ev.result.reason}`
                  : "Allowed" }}
              </Badge>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </CardContent>
  </Card>
</template>
