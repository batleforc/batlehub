<script setup lang="ts">
import { ref, computed } from "vue";
import { listRegistries, myNamespaces as myNamespacesApi } from "@/client/sdk.gen";
import type { RegistryInfo } from "@/client/types.gen";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import type { TeamNamespaceDto } from "@/lib/registry-types";
import { Badge } from "@/components/ui/badge";
import { Card, CardHeader, CardTitle, CardContent, CardDescription } from "@/components/ui/card";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";
import NamespacePackagesTable from "@/components/namespace/NamespacePackagesTable.vue";
import NamespaceUpload from "@/components/namespace/NamespaceUpload.vue";

const { token, identity } = useAuth();

const groups = computed(() => identity.value?.groups ?? []);
const hasGroups = computed(() => groups.value.length > 0);

const { data: registriesData } = useApi<RegistryInfo[]>(
  () => listRegistries() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

const {
  data: myNamespaces,
  error: namespacesError,
  loading: namespacesLoading,
} = useApi<TeamNamespaceDto[]>(() => {
  if (!token.value) return Promise.resolve({ data: [] });
  return myNamespacesApi() as Promise<{ data?: unknown; error?: unknown }>;
}, [token]);

const selectedNs = ref<TeamNamespaceDto | null>(null);

function selectNamespace(ns: TeamNamespaceDto) {
  selectedNs.value = ns;
}
</script>

<template>
  <div class="space-y-6 max-w-4xl">
    <div>
      <h1 class="font-mono text-2xl font-bold cyber-text-glow">Team Namespace</h1>
      <p class="text-sm text-muted-foreground mt-0.5">
        View and manage the packages and namespaces owned by your groups.
      </p>
    </div>

    <Card v-if="!hasGroups">
      <CardContent class="pt-6">
        <p class="text-sm text-muted-foreground">
          You are not a member of any groups. Contact your administrator to be added to a team
          namespace.
        </p>
      </CardContent>
    </Card>

    <template v-else>
      <!-- Groups -->
      <Card>
        <CardHeader><CardTitle class="text-base">Your groups</CardTitle></CardHeader>
        <CardContent>
          <div class="flex flex-wrap gap-2">
            <Badge v-for="g in groups" :key="g" variant="secondary" class="font-mono text-xs">
              {{ g.replaceAll(" ", "") }}
            </Badge>
          </div>
        </CardContent>
      </Card>

      <!-- Namespaces list -->
      <Card>
        <CardHeader>
          <CardTitle class="text-base">My namespaces</CardTitle>
          <CardDescription>Click a row to browse its packages.</CardDescription>
        </CardHeader>
        <CardContent>
          <p v-if="namespacesLoading" class="text-sm text-muted-foreground">Loading…</p>
          <p v-else-if="namespacesError" class="text-sm text-destructive">{{ namespacesError }}</p>
          <p v-else-if="!myNamespaces?.length" class="text-sm text-muted-foreground">
            No namespace claims found for your groups.
          </p>
          <Table v-else>
            <TableHeader>
              <TableRow>
                <TableHead>Registry</TableHead>
                <TableHead>Prefix</TableHead>
                <TableHead>Group</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow
                v-for="ns in myNamespaces"
                :key="`${ns.registry}:${ns.prefix}`"
                class="cursor-pointer"
                :class="
                  selectedNs?.registry === ns.registry && selectedNs?.prefix === ns.prefix
                    ? 'bg-muted/60'
                    : 'hover:bg-muted/40'
                "
                @click="selectNamespace(ns)"
              >
                <TableCell class="font-mono text-xs">{{ ns.registry }}</TableCell>
                <TableCell class="font-mono text-xs">{{ ns.prefix }}</TableCell>
                <TableCell class="font-mono text-xs">{{
                  ns.group_id.replaceAll(" ", "")
                }}</TableCell>
              </TableRow>
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      <!-- Packages for selected namespace -->
      <Card>
        <CardHeader>
          <CardTitle class="text-base">
            Packages
            <span
              v-if="selectedNs"
              class="ml-2 font-mono text-muted-foreground text-sm font-normal"
            >
              {{ selectedNs.registry }} / {{ selectedNs.prefix }}
            </span>
          </CardTitle>
          <CardDescription>
            {{
              selectedNs
                ? "Published versions under the selected namespace."
                : "Select a namespace row above to browse its packages."
            }}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <NamespacePackagesTable v-if="selectedNs" :namespace="selectedNs" />
        </CardContent>
      </Card>

      <!-- Upload -->
      <Card>
        <CardHeader>
          <CardTitle class="text-base">Upload package</CardTitle>
          <CardDescription>Publish a new package to one of your registries.</CardDescription>
        </CardHeader>
        <CardContent class="space-y-4">
          <NamespaceUpload :registries="registriesData ?? []" />
        </CardContent>
      </Card>
    </template>
  </div>
</template>
