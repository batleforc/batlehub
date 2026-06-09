<script setup lang="ts">
import { ref } from "vue";
import { useAuthFetch } from "@/composables/useAuthFetch";
import { API_BASE_URL } from "@/config";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

const { authFetch } = useAuthFetch();

// ── Filter state ──────────────────────────────────────────────────────────────

const registry = ref("");
const fromDate = ref("");
const toDate = ref("");
const format = ref<"spdx" | "cyclonedx">("spdx");

const loading = ref(false);
const errorMsg = ref<string | null>(null);

// ── Helpers ───────────────────────────────────────────────────────────────────

async function downloadBlob(url: string, defaultFilename: string) {
  const resp = await authFetch(`${API_BASE_URL}${url}`);
  if (!resp.ok) throw new Error(await resp.text().catch(() => `HTTP ${resp.status}`));
  const disposition = resp.headers.get("Content-Disposition") ?? "";
  const match = disposition.match(/filename="([^"]+)"/);
  const filename = match?.[1] ?? defaultFilename;
  const blob = await resp.blob();
  const a = Object.assign(document.createElement("a"), {
    href: URL.createObjectURL(blob),
    download: filename,
  });
  a.click();
  URL.revokeObjectURL(a.href);
}

// ── Export action ─────────────────────────────────────────────────────────────

async function exportSbom() {
  loading.value = true;
  errorMsg.value = null;
  try {
    const params = new URLSearchParams({ format: format.value });
    if (registry.value.trim()) params.set("registry", registry.value.trim());
    if (fromDate.value) params.set("from", `${fromDate.value}T00:00:00Z`);
    if (toDate.value) params.set("to", `${toDate.value}T23:59:59Z`);

    const ext = format.value === "cyclonedx" ? "cyclonedx.json" : "spdx.json";
    const ts = new Date().toISOString().slice(0, 10).replaceAll("-", "");
    const label = registry.value.trim() || "all";
    await downloadBlob(
      `/api/v1/sbom/export?${params.toString()}`,
      `sbom-export-${label}-${ts}.${ext}`,
    );
  } catch (e: unknown) {
    errorMsg.value = e instanceof Error ? e.message : "Export failed";
  } finally {
    loading.value = false;
  }
}
</script>

<template>
  <div class="space-y-6">
    <h1 class="font-mono text-2xl font-bold cyber-text-glow">SBOM Export</h1>

    <!-- Feedback -->
    <div
      v-if="errorMsg"
      class="rounded-sm bg-destructive/10 border border-destructive/30 px-4 py-2 text-destructive text-sm"
    >
      {{ errorMsg }}
    </div>

    <!-- Export card -->
    <Card>
      <CardHeader>
        <CardTitle>Export Org-Level SBOM</CardTitle>
      </CardHeader>
      <CardContent class="space-y-4">
        <p class="text-sm text-muted-foreground">
          Generates a merged SBOM covering all artifacts served in the selected time window. Leave
          filters empty to export everything.
        </p>

        <div class="grid grid-cols-1 sm:grid-cols-2 gap-4 max-w-xl">
          <!-- Registry filter -->
          <div class="space-y-1.5">
            <Label for="sbom-registry"
              >Registry <span class="text-muted-foreground font-normal">(optional)</span></Label
            >
            <Input id="sbom-registry" v-model="registry" placeholder="e.g. crates-io" />
          </div>

          <!-- Format -->
          <div class="space-y-1.5">
            <Label for="sbom-format">Format</Label>
            <select
              id="sbom-format"
              v-model="format"
              class="w-full border border-input rounded-sm px-2 py-2 font-mono text-sm bg-background focus:outline-none focus:ring-2 focus:ring-ring"
            >
              <option value="spdx">SPDX 2.3</option>
              <option value="cyclonedx">CycloneDX 1.4</option>
            </select>
          </div>

          <!-- From date -->
          <div class="space-y-1.5">
            <Label for="sbom-from"
              >From <span class="text-muted-foreground font-normal">(optional)</span></Label
            >
            <Input id="sbom-from" v-model="fromDate" type="date" />
          </div>

          <!-- To date -->
          <div class="space-y-1.5">
            <Label for="sbom-to"
              >To <span class="text-muted-foreground font-normal">(optional)</span></Label
            >
            <Input id="sbom-to" v-model="toDate" type="date" />
          </div>
        </div>

        <Button :disabled="loading" @click="exportSbom">
          {{ loading ? "Exporting…" : "Download SBOM" }}
        </Button>
      </CardContent>
    </Card>

    <!-- About card -->
    <Card>
      <CardHeader>
        <CardTitle class="text-base">About SBOM Formats</CardTitle>
      </CardHeader>
      <CardContent class="text-sm text-muted-foreground space-y-2">
        <p>
          <strong class="text-foreground">SPDX 2.3</strong> — ISO/IEC standard widely used for
          compliance and license tracking. Preferred for legal review and OpenChain-conformant
          workflows.
        </p>
        <p>
          <strong class="text-foreground">CycloneDX 1.4</strong> — OWASP standard optimised for
          security tooling. Preferred for vulnerability scanning and SBOM-driven dependency
          analysis.
        </p>
        <p>
          Per-artifact SBOMs (SPDX or CycloneDX) are also available from the
          <RouterLink to="/explore" class="underline hover:text-foreground"
            >Package Explorer</RouterLink
          >
          version detail view.
        </p>
      </CardContent>
    </Card>
  </div>
</template>
