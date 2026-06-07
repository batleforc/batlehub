<script setup lang="ts">
import type { PackageEventDto } from "@/client/types.gen";
import { Badge } from "@/components/ui/badge";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";

defineProps<{ events: PackageEventDto[] }>();

function fmtDate(iso: string) {
  return new Date(iso).toLocaleString();
}

const ACTION_LABELS: Record<string, string> = {
  download: "Download",
  view_metadata: "View metadata",
  block: "Block",
  unblock: "Unblock",
};
function fmtAction(a: string) {
  return ACTION_LABELS[a] ?? a;
}
</script>

<template>
  <Card>
    <CardHeader>
      <CardTitle class="text-base">Recent access events</CardTitle>
    </CardHeader>
    <CardContent class="p-0">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>When</TableHead>
            <TableHead>User</TableHead>
            <TableHead>Role</TableHead>
            <TableHead>Version</TableHead>
            <TableHead>Artifact</TableHead>
            <TableHead>Action</TableHead>
            <TableHead>Outcome</TableHead>
            <TableHead>Reason</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableRow v-for="ev in events" :key="ev.id">
            <TableCell class="text-xs tabular-nums whitespace-nowrap">{{
              fmtDate(ev.timestamp)
            }}</TableCell>
            <TableCell class="text-sm">
              <span v-if="ev.user_id">{{ ev.user_id }}</span>
              <span v-else class="text-muted-foreground italic">anonymous</span>
            </TableCell>
            <TableCell>
              <Badge variant="outline" class="text-xs capitalize">{{ ev.user_role }}</Badge>
            </TableCell>
            <TableCell class="font-mono text-xs">{{ ev.version }}</TableCell>
            <TableCell class="font-mono text-xs text-muted-foreground">{{
              ev.artifact ?? "—"
            }}</TableCell>
            <TableCell class="text-xs">{{ fmtAction(ev.action) }}</TableCell>
            <TableCell>
              <Badge
                :variant="ev.outcome === 'denied' ? 'destructive' : 'secondary'"
                class="text-xs"
              >
                {{ ev.outcome }}
              </Badge>
            </TableCell>
            <TableCell
              class="text-xs text-muted-foreground max-w-[200px] truncate"
              :title="ev.deny_reason ?? ''"
            >
              {{ ev.deny_reason ?? "—" }}
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
      <p v-if="events.length === 0" class="p-6 text-sm text-muted-foreground text-center">
        No events recorded yet.
      </p>
    </CardContent>
  </Card>
</template>
