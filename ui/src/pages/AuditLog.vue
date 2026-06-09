<script setup lang="ts">
import { ref, computed } from "vue";
import { auditLog } from "@/client/sdk.gen";
import type { PackageIdentifierDto } from "@/client/types.gen";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";

interface AccessEvent {
  id: string;
  package_id: PackageIdentifierDto;
  user_id?: string;
  user_role: string;
  action: string;
  result: { outcome: "allowed" } | { outcome: "denied"; reason: string };
  timestamp: string;
}

const { token } = useAuth();

const { data, error, loading, reload } = useApi<AccessEvent[]>(
  () => auditLog() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

const userFilter = ref("");
const actionFilter = ref("");

const filteredItems = computed(() => {
  if (!data.value?.length) return [];
  return data.value.filter((ev) => {
    const uq = userFilter.value.toLowerCase().trim();
    const aq = actionFilter.value.toLowerCase().trim();
    if (uq && !(ev.user_id ?? "").toLowerCase().includes(uq)) return false;
    if (aq && !ev.action.toLowerCase().includes(aq)) return false;
    return true;
  });
});

const actionOptions = computed(() => {
  if (!data.value?.length) return [];
  return [...new Set(data.value.map((e) => e.action))].sort();
});
</script>

<template>
  <Card>
    <CardHeader class="space-y-3 pb-3">
      <div class="flex flex-row items-center justify-between space-y-0">
        <CardTitle class="text-lg">
          Audit Log
          <span v-if="data?.length" class="font-normal text-muted-foreground text-base ml-1">
            ({{ data.length }})
          </span>
        </CardTitle>
        <Button variant="outline" size="sm" @click="reload"> Refresh </Button>
      </div>
      <div class="flex gap-2 flex-wrap">
        <Input
          v-model="userFilter"
          placeholder="Filter by user…"
          class="h-8 text-sm max-w-[200px]"
        />
        <select
          v-model="actionFilter"
          aria-label="Filter by action"
          class="h-8 rounded-sm border border-input bg-transparent px-2 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring text-foreground"
        >
          <option value="">All actions</option>
          <option v-for="a in actionOptions" :key="a" :value="a">
            {{ a }}
          </option>
        </select>
      </div>
    </CardHeader>
    <CardContent class="p-0">
      <p v-if="loading" class="p-6 text-sm text-muted-foreground">Loading…</p>
      <p v-else-if="error" class="p-6 text-sm text-destructive">
        {{ error }}
      </p>

      <Table v-else-if="filteredItems.length">
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
            v-for="ev in filteredItems"
            :key="ev.id"
            :class="ev.result.outcome === 'denied' ? 'bg-destructive/5' : ''"
          >
            <TableCell class="whitespace-nowrap text-xs tabular-nums">
              {{ new Date(ev.timestamp).toLocaleString() }}
            </TableCell>
            <TableCell class="text-sm font-mono">
              <span v-if="ev.user_id">{{ ev.user_id }}</span>
              <span v-else class="text-muted-foreground italic not-italic font-sans"
                >anonymous</span
              >
            </TableCell>
            <TableCell class="font-mono text-xs">
              {{ ev.package_id.registry }}
            </TableCell>
            <TableCell class="font-mono text-xs">
              {{ ev.package_id.name }}@{{ ev.package_id.version }}
              <span v-if="ev.package_id.artifact" class="text-muted-foreground">
                ({{ ev.package_id.artifact }})
              </span>
            </TableCell>
            <TableCell class="text-xs font-mono">
              {{ ev.action }}
            </TableCell>
            <TableCell class="max-w-[220px]">
              <Badge :variant="ev.result.outcome === 'denied' ? 'destructive' : 'secondary'">
                {{ ev.result.outcome === "denied" ? "Denied" : "Allowed" }}
              </Badge>
              <p
                v-if="ev.result.outcome === 'denied'"
                class="mt-0.5 text-xs text-muted-foreground truncate"
                :title="ev.result.reason"
              >
                {{ ev.result.reason }}
              </p>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>

      <div v-else-if="!loading" class="p-6 text-sm text-muted-foreground text-center">
        {{
          userFilter || actionFilter
            ? "No events match the current filters."
            : "No events recorded yet."
        }}
      </div>
    </CardContent>
  </Card>
</template>
