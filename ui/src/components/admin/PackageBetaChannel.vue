<script setup lang="ts">
import { ref } from "vue";
import { listBetaMembers } from "@/client/sdk.gen";
import type { BetaChannelMemberDto } from "@/lib/registry-types";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardHeader, CardContent } from "@/components/ui/card";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";

const props = defineProps<{ registry: string }>();

const { token } = useAuth();
const expanded = ref(false);

const {
  data: members,
  loading,
  reload,
} = useApi<BetaChannelMemberDto[]>(() => {
  if (!props.registry) return Promise.resolve({ data: [] });
  return listBetaMembers({ path: { registry: props.registry } }) as Promise<{
    data?: unknown;
    error?: unknown;
  }>;
}, [token]);
</script>

<template>
  <Card>
    <CardHeader>
      <div class="flex items-center justify-between">
        <button
          class="flex items-center gap-2 text-base font-semibold hover:text-primary transition-colors"
          @click="expanded = !expanded"
        >
          Beta Channel Access
          <span class="text-muted-foreground text-xs font-normal">{{
            expanded ? "▲ hide" : "▼ show"
          }}</span>
          <Badge v-if="members && members.length > 0" variant="secondary" class="text-xs ml-1">
            {{ members.length }} member{{ members.length > 1 ? "s" : "" }}
          </Badge>
        </button>
        <Button v-if="expanded" variant="outline" size="sm" :disabled="loading" @click="reload">
          {{ loading ? "Loading…" : "Refresh" }}
        </Button>
      </div>
    </CardHeader>
    <CardContent v-if="expanded" class="p-0">
      <p class="px-6 py-2 text-xs text-muted-foreground border-b">
        Pre-release versions are only accessible to the users and groups listed here.
      </p>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Type</TableHead>
            <TableHead>Principal ID</TableHead>
            <TableHead>Granted by</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <TableRow v-for="m in members" :key="m.principal_type + ':' + m.principal_id">
            <TableCell>
              <Badge
                :variant="m.principal_type === 'user' ? 'default' : 'secondary'"
                class="text-xs capitalize"
              >
                {{ m.principal_type }}
              </Badge>
            </TableCell>
            <TableCell class="font-mono text-sm">{{ m.principal_id }}</TableCell>
            <TableCell class="text-sm text-muted-foreground">{{ m.granted_by ?? "—" }}</TableCell>
          </TableRow>
        </TableBody>
      </Table>
      <p
        v-if="!members || members.length === 0"
        class="p-6 text-sm text-muted-foreground text-center"
      >
        No beta channel members — pre-release versions are not accessible to anyone.
      </p>
    </CardContent>
  </Card>
</template>
