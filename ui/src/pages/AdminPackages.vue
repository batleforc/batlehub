<script setup lang="ts">
import { ref } from "vue";
import { useRouter } from "vue-router";
import { listPackages, blockPackage, unblockPackage } from "@/client/sdk.gen";
import { useApi } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell,
} from "@/components/ui/table";

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
  last_accessed_by: string | null;
  access_count: number;
}

const { token } = useAuth();
const router = useRouter();

const { data: packages, error, loading, reload } = useApi<AdminPackageSummary[]>(
  () => listPackages() as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

// ── Block existing package ────────────────────────────────────────────────────

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

// ── Pre-block form ────────────────────────────────────────────────────────────

const showPreBlock = ref(false);
const preBlock = ref({
  registry: "github",
  name: "",
  version: "",
  artifact: "",
  reason: "",
});
const preBlockError = ref<string | null>(null);
const preBlockLoading = ref(false);

async function submitPreBlock() {
  if (!preBlock.value.name || !preBlock.value.version || !preBlock.value.reason) {
    preBlockError.value = "Name, version and reason are required.";
    return;
  }
  preBlockError.value = null;
  preBlockLoading.value = true;
  try {
    await blockPackage({
      body: {
        registry: preBlock.value.registry,
        name: preBlock.value.name,
        version: preBlock.value.version,
        artifact: preBlock.value.artifact || undefined,
        reason: preBlock.value.reason,
      },
    });
    preBlock.value = { registry: "github", name: "", version: "", artifact: "", reason: "" };
    showPreBlock.value = false;
    reload();
  } catch (e: unknown) {
    preBlockError.value = e instanceof Error ? e.message : "Failed to block package.";
  } finally {
    preBlockLoading.value = false;
  }
}
</script>

<template>
  <div class="space-y-4">
    <!-- Pre-block form -->
    <Card>
      <CardHeader class="flex flex-row items-center justify-between space-y-0 pb-3">
        <CardTitle class="text-base">Block a package</CardTitle>
        <Button variant="outline" size="sm" @click="showPreBlock = !showPreBlock">
          {{ showPreBlock ? "Cancel" : "Block new package" }}
        </Button>
      </CardHeader>

      <CardContent v-if="showPreBlock" class="space-y-4 pt-0">
        <p class="text-xs text-muted-foreground">
          Pre-emptively block a package before it is downloaded. The block takes
          effect immediately — any subsequent request for that package will be denied.
        </p>

        <div class="grid grid-cols-2 gap-3 sm:grid-cols-4">
          <div class="space-y-1">
            <Label for="pb-registry">Registry</Label>
            <select
              id="pb-registry"
              v-model="preBlock.registry"
              class="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            >
              <option value="github">github</option>
              <option value="npm">npm</option>
              <option value="cargo">cargo</option>
            </select>
          </div>
          <div class="space-y-1 sm:col-span-2">
            <Label for="pb-name">Name</Label>
            <Input id="pb-name" v-model="preBlock.name" placeholder="owner/repo or lodash or serde" class="font-mono" />
          </div>
          <div class="space-y-1">
            <Label for="pb-version">Version / tag</Label>
            <Input id="pb-version" v-model="preBlock.version" placeholder="v1.2.3" class="font-mono" />
          </div>
        </div>

        <div class="grid grid-cols-2 gap-3">
          <div class="space-y-1">
            <Label for="pb-artifact">Artifact <span class="text-muted-foreground">(optional)</span></Label>
            <Input id="pb-artifact" v-model="preBlock.artifact" placeholder="tarball / 123456789 / download" class="font-mono" />
          </div>
          <div class="space-y-1">
            <Label for="pb-reason">Reason</Label>
            <Input id="pb-reason" v-model="preBlock.reason" placeholder="CVE-2025-XXXX or policy violation" />
          </div>
        </div>

        <p v-if="preBlockError" class="text-xs text-destructive">{{ preBlockError }}</p>

        <Button :disabled="preBlockLoading" @click="submitPreBlock">
          {{ preBlockLoading ? "Blocking…" : "Block package" }}
        </Button>
      </CardContent>
    </Card>

    <!-- Package list -->
    <Card>
      <CardHeader class="flex flex-row items-center justify-between space-y-0 pb-4">
        <CardTitle class="text-lg">All packages</CardTitle>
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
              <TableHead>Last pulled by</TableHead>
              <TableHead class="text-right">Downloads</TableHead>
              <TableHead></TableHead>
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
              <TableCell class="text-muted-foreground text-xs font-mono">{{ pkg.package_id.artifact ?? "—" }}</TableCell>
              <TableCell>
                <div class="space-y-0.5">
                  <Badge :variant="pkg.status.status === 'blocked' ? 'destructive' : 'secondary'">
                    {{ pkg.status.status === "blocked" ? "Blocked" : "Available" }}
                  </Badge>
                  <p v-if="pkg.status.status === 'blocked'" class="text-xs text-muted-foreground">
                    {{ pkg.status.reason }}
                  </p>
                </div>
              </TableCell>
              <TableCell class="text-sm">
                <span v-if="pkg.last_accessed_by" class="font-medium">{{ pkg.last_accessed_by }}</span>
                <span v-else-if="pkg.access_count > 0" class="text-muted-foreground italic">anonymous</span>
                <span v-else class="text-muted-foreground">—</span>
              </TableCell>
              <TableCell class="text-right tabular-nums">{{ pkg.access_count }}</TableCell>
              <TableCell>
                <Button
                  variant="ghost"
                  size="sm"
                  @click="router.push({ path: '/admin/packages/detail', query: { registry: pkg.package_id.registry, name: pkg.package_id.name } })"
                >
                  Details
                </Button>
              </TableCell>
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

        <p v-else class="p-6 text-sm text-muted-foreground text-center">No packages yet.</p>
      </CardContent>
    </Card>
  </div>
</template>
