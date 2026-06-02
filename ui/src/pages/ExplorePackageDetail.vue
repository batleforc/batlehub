<script setup lang="ts">
import { ref, computed, onMounted } from "vue";
import { useRoute, useRouter } from "vue-router";
import { ArrowLeft, ShieldCheck, ShieldAlert, Lock, Unlock, Package, FileJson, FileCode } from "@lucide/vue";
import { useAuth } from "@/composables/useAuth";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table, TableHeader, TableHead, TableBody, TableRow, TableCell,
} from "@/components/ui/table";

interface GateDto {
  registry_accessible: boolean;
  beta_member: boolean;
}

interface FirewallClear { status: "clear" }
interface FirewallBlocked { status: "blocked"; reason: string; blocked_by: string; blocked_at: string }
interface FirewallYanked { status: "yanked" }
type FirewallDto = FirewallClear | FirewallBlocked | FirewallYanked;

interface ExploreVersionDto {
  version: string;
  source: "proxied" | "local";
  firewall: FirewallDto;
  download_count: number;
  last_accessed: string | null;
  published_at: string | null;
  is_prerelease: boolean;
}

interface ExplorePackageDetailResponse {
  registry: string;
  name: string;
  gate: GateDto;
  versions: ExploreVersionDto[];
}

const API_BASE = import.meta.env.VITE_API_BASE_URL ?? "";
const { token } = useAuth();
const route = useRoute();
const router = useRouter();

const registry = computed(() => String(route.params.registry ?? ""));
const name = computed(() => String(route.params.name ?? ""));

const data = ref<ExplorePackageDetailResponse | null>(null);
const loading = ref(false);
const error = ref<string | null>(null);

function headers() {
  const h: Record<string, string> = {};
  if (token.value) h["Authorization"] = `Bearer ${token.value}`;
  return h;
}

// ── Per-artifact SBOM download ─────────────────────────────────────────────

const sbomLoading = ref<string | null>(null); // "registry/name/version:format"
const sbomMissing = ref<Set<string>>(new Set());

async function downloadSbom(version: string, fmt: "spdx" | "cyclonedx") {
  const key = `${registry.value}/${name.value}/${version}:${fmt}`;
  sbomLoading.value = key;
  try {
    const ext = fmt === "cyclonedx" ? "cyclonedx.json" : "spdx.json";
    const url = `/api/v1/sbom/${encodeURIComponent(registry.value)}/${encodeURIComponent(name.value)}/${encodeURIComponent(version)}?format=${fmt}`;
    const resp = await fetch(`${API_BASE}${url}`, { headers: headers() });
    if (resp.status === 404) {
      sbomMissing.value = new Set([...sbomMissing.value, `${registry.value}/${name.value}/${version}`]);
      return;
    }
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    const disposition = resp.headers.get("Content-Disposition") ?? "";
    const match = disposition.match(/filename="([^"]+)"/);
    const filename = match?.[1] ?? `${name.value}-${version}.${ext}`;
    const blob = await resp.blob();
    const a = Object.assign(document.createElement("a"), {
      href: URL.createObjectURL(blob),
      download: filename,
    });
    a.click();
    URL.revokeObjectURL(a.href);
  } catch {
    // silently ignore download errors
  } finally {
    sbomLoading.value = null;
  }
}

async function fetchDetail() {
  loading.value = true;
  error.value = null;
  try {
    const url = `${API_BASE}/api/v1/explore/packages/${encodeURIComponent(registry.value)}/${encodeURIComponent(name.value)}`;
    const res = await fetch(url, { headers: headers() });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    data.value = await res.json();
  } catch (e) {
    error.value = e instanceof Error ? e.message : "Failed to load package detail";
  } finally {
    loading.value = false;
  }
}

function goBack() {
  router.push({
    path: "/explore",
    query: { registry: registry.value },
  });
}

function firewallVariant(fw: FirewallDto): "default" | "destructive" | "secondary" | "outline" {
  if (fw.status === "blocked") return "destructive";
  if (fw.status === "yanked") return "secondary";
  return "outline";
}

function firewallLabel(fw: FirewallDto) {
  if (fw.status === "blocked") return "Blocked";
  if (fw.status === "yanked") return "Yanked";
  return "Clear";
}

function formatDate(iso: string | null) {
  if (!iso) return "—";
  return new Date(iso).toLocaleDateString(undefined, {
    year: "numeric", month: "short", day: "numeric",
  });
}

onMounted(fetchDetail);
</script>

<template>
  <div class="space-y-6 max-w-4xl">
    <!-- Back link -->
    <button
      class="flex items-center gap-1.5 text-sm text-muted-foreground hover:text-foreground transition-colors"
      @click="goBack"
    >
      <ArrowLeft class="h-4 w-4" />
      Back to Explorer
    </button>

    <template v-if="loading">
      <p class="text-muted-foreground text-sm">Loading…</p>
    </template>

    <template v-else-if="error">
      <p class="text-destructive text-sm">{{ error }}</p>
    </template>

    <template v-else-if="data">
      <!-- Header -->
      <div class="flex items-start gap-3 flex-wrap">
        <div class="flex-1">
          <div class="flex items-center gap-2 flex-wrap">
            <Package class="h-6 w-6 text-primary shrink-0" />
            <h1 class="text-2xl font-bold font-mono">{{ data.name }}</h1>
            <Badge variant="outline">{{ data.registry }}</Badge>
          </div>
          <p class="text-sm text-muted-foreground mt-1">
            {{ data.versions.length }} known version{{ data.versions.length !== 1 ? "s" : "" }}
          </p>
        </div>
        <Button variant="outline" size="sm" @click="fetchDetail">
          Refresh
        </Button>
      </div>

      <!-- Gate summary card -->
      <Card>
        <CardHeader class="pb-2">
          <CardTitle class="text-base">Access Gate</CardTitle>
        </CardHeader>
        <CardContent>
          <div class="space-y-2">
            <!-- Registry access -->
            <div class="flex items-center gap-2 text-sm">
              <component
                :is="data.gate.registry_accessible ? ShieldCheck : ShieldAlert"
                :class="data.gate.registry_accessible ? 'text-green-500' : 'text-destructive'"
                class="h-4 w-4 shrink-0"
              />
              <span class="text-muted-foreground">Registry access:</span>
              <span :class="data.gate.registry_accessible ? 'text-green-600 font-medium' : 'text-destructive font-medium'">
                {{ data.gate.registry_accessible ? "Allowed" : "Denied" }}
              </span>
            </div>

            <!-- Beta channel -->
            <div class="flex items-center gap-2 text-sm">
              <component
                :is="data.gate.beta_member ? Unlock : Lock"
                :class="data.gate.beta_member ? 'text-green-500' : 'text-muted-foreground'"
                class="h-4 w-4 shrink-0"
              />
              <span class="text-muted-foreground">Beta channel:</span>
              <span :class="data.gate.beta_member ? 'text-green-600 font-medium' : 'text-muted-foreground'">
                {{ data.gate.beta_member ? "Member — pre-release versions visible" : "Non-member" }}
              </span>
            </div>
          </div>
        </CardContent>
      </Card>

      <!-- Versions table -->
      <Card>
        <CardHeader class="pb-2">
          <CardTitle class="text-base">Versions</CardTitle>
        </CardHeader>
        <CardContent class="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Version</TableHead>
                <TableHead>Source</TableHead>
                <TableHead>Firewall</TableHead>
                <TableHead class="text-right">Downloads</TableHead>
                <TableHead>Last Accessed</TableHead>
                <TableHead>Published</TableHead>
                <TableHead v-if="token">SBOM</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              <TableRow
                v-for="ver in data.versions"
                :key="`${ver.version}-${ver.source}`"
                :class="ver.is_prerelease ? 'text-muted-foreground italic' : ''"
              >
                <TableCell class="font-mono text-sm">
                  {{ ver.version }}
                  <Badge
                    v-if="ver.is_prerelease"
                    variant="outline"
                    class="ml-1 text-xs"
                  >
                    pre-release
                  </Badge>
                </TableCell>
                <TableCell>
                  <Badge
                    :variant="ver.source === 'local' ? 'secondary' : 'outline'"
                    class="text-xs"
                  >
                    {{ ver.source === "local" ? "Local" : "Proxied" }}
                  </Badge>
                </TableCell>
                <TableCell>
                  <span
                    v-if="ver.firewall.status === 'blocked'"
                    class="group relative"
                  >
                    <Badge variant="destructive" class="text-xs cursor-help">Blocked</Badge>
                    <span
                      class="absolute bottom-full left-0 mb-1 hidden group-hover:block z-10 w-64 rounded bg-popover border p-2 text-xs text-popover-foreground shadow-md"
                    >
                      <strong>Reason:</strong> {{ (ver.firewall as any).reason }}<br />
                      <strong>By:</strong> {{ (ver.firewall as any).blocked_by }}<br />
                      <strong>At:</strong> {{ formatDate((ver.firewall as any).blocked_at) }}
                    </span>
                  </span>
                  <Badge
                    v-else
                    :variant="firewallVariant(ver.firewall)"
                    class="text-xs"
                  >
                    {{ firewallLabel(ver.firewall) }}
                  </Badge>
                </TableCell>
                <TableCell class="text-right text-sm text-muted-foreground">
                  {{ ver.download_count.toLocaleString() }}
                </TableCell>
                <TableCell class="text-sm text-muted-foreground">
                  {{ formatDate(ver.last_accessed) }}
                </TableCell>
                <TableCell class="text-sm text-muted-foreground">
                  {{ formatDate(ver.published_at) }}
                </TableCell>
                <TableCell v-if="token" class="text-sm">
                  <span
                    v-if="sbomMissing.has(`${registry}/${name}/${ver.version}`)"
                    class="text-muted-foreground text-xs"
                  >
                    No SBOM
                  </span>
                  <div v-else class="flex gap-1">
                    <button
                      :disabled="sbomLoading === `${registry}/${name}/${ver.version}:spdx`"
                      class="inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-xs hover:bg-accent disabled:opacity-50"
                      title="Download SPDX 2.3"
                      @click="downloadSbom(ver.version, 'spdx')"
                    >
                      <FileJson class="h-3 w-3" />
                      SPDX
                    </button>
                    <button
                      :disabled="sbomLoading === `${registry}/${name}/${ver.version}:cyclonedx`"
                      class="inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-xs hover:bg-accent disabled:opacity-50"
                      title="Download CycloneDX 1.4"
                      @click="downloadSbom(ver.version, 'cyclonedx')"
                    >
                      <FileCode class="h-3 w-3" />
                      CDX
                    </button>
                  </div>
                </TableCell>
              </TableRow>
              <TableRow v-if="data.versions.length === 0">
                <TableCell :colspan="token ? 7 : 6" class="text-center text-muted-foreground py-6">
                  No versions found
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </template>
  </div>
</template>
