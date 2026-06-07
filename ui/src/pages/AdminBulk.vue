<script setup lang="ts">
import { ref, computed } from "vue";
import { Upload, CheckCircle2, XCircle } from "@lucide/vue";
import { bulkBlockPackages, bulkUnblockPackages } from "@/client/sdk.gen";
import type {
  BulkActionResponse,
  BulkBlockRequestItem,
  BulkUnblockRequestItem,
} from "@/client/types.gen";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
} from "@/components/ui/table";

type Action = "block" | "unblock";

interface ParsedRow {
  registry: string;
  name: string;
  version: string;
  artifact: string;
  reason: string;
  error?: string;
}

const action = ref<Action>("block");
const csvText = ref("");
const defaultReason = ref("");
const parsedRows = ref<ParsedRow[]>([]);
const parseError = ref<string | null>(null);
const submitting = ref(false);
const result = ref<BulkActionResponse | null>(null);
const submitError = ref<string | null>(null);

const validRows = computed(() => parsedRows.value.filter((r) => !r.error));
const invalidRows = computed(() => parsedRows.value.filter((r) => !!r.error));

function parseCSV() {
  result.value = null;
  submitError.value = null;
  parseError.value = null;
  parsedRows.value = [];

  const text = csvText.value.trim();
  if (!text) {
    parseError.value = "Paste some CSV content first.";
    return;
  }

  const lines = text.split(/\r?\n/);
  // Skip header if present
  const dataLines = lines[0].toLowerCase().startsWith("registry") ? lines.slice(1) : lines;

  if (dataLines.length === 0 || (dataLines.length === 1 && !dataLines[0].trim())) {
    parseError.value = "No data rows found.";
    return;
  }

  parsedRows.value = dataLines
    .filter((l) => l.trim())
    .map((line) => {
      const cols = line.split(",").map((c) => c.trim());
      const [registry = "", name = "", version = "", artifact = "", reason = ""] = cols;
      const row: ParsedRow = { registry, name, version, artifact, reason };

      if (!registry) row.error = "registry is required";
      else if (!name) row.error = "name is required";
      else if (!version) row.error = "version is required";
      else if (action.value === "block" && !reason && !defaultReason.value) {
        row.error = "reason is required for block (set per-row or use default reason)";
      }

      return row;
    });
}

function handleFileUpload(event: Event) {
  const file = (event.target as HTMLInputElement).files?.[0];
  if (!file) return;
  const reader = new FileReader();
  reader.onload = (e) => {
    csvText.value = (e.target?.result as string) ?? "";
    parseCSV();
  };
  reader.readAsText(file);
}

async function submit() {
  if (validRows.value.length === 0) return;
  submitting.value = true;
  submitError.value = null;
  result.value = null;

  try {
    if (action.value === "block") {
      const items: BulkBlockRequestItem[] = validRows.value.map((r) => ({
        registry: r.registry,
        name: r.name,
        version: r.version,
        artifact: r.artifact || null,
        reason: r.reason || defaultReason.value,
      }));
      const res = await bulkBlockPackages({ body: { items } });
      result.value = res.data ?? null;
    } else {
      const items: BulkUnblockRequestItem[] = validRows.value.map((r) => ({
        registry: r.registry,
        name: r.name,
        version: r.version,
        artifact: r.artifact || null,
      }));
      const res = await bulkUnblockPackages({ body: { items } });
      result.value = res.data ?? null;
    }
  } catch (e) {
    submitError.value = e instanceof Error ? e.message : "Request failed.";
  } finally {
    submitting.value = false;
  }
}

function reset() {
  csvText.value = "";
  parsedRows.value = [];
  result.value = null;
  parseError.value = null;
  submitError.value = null;
  defaultReason.value = "";
}
</script>

<template>
  <div class="space-y-6 max-w-4xl">
    <div>
      <h1 class="font-mono text-xl font-bold cyber-text-glow">Bulk Import</h1>
      <p class="text-sm text-muted-foreground mt-1">
        Block or unblock multiple packages at once by pasting or uploading a CSV file.
      </p>
    </div>

    <!-- Format reference -->
    <Card>
      <CardHeader class="pb-2">
        <CardTitle class="text-sm font-medium text-muted-foreground"> CSV format </CardTitle>
      </CardHeader>
      <CardContent>
        <pre class="text-xs font-mono bg-muted rounded p-3 overflow-x-auto">
registry,name,version,artifact,reason
npm,lodash,4.17.21,,CVE-2021-23337
cargo,serde,1.0.0,,License issue
github,org/repo,v2.0.0,binary.tar.gz,Supply chain risk</pre
        >
        <p class="text-xs text-muted-foreground mt-2">
          Header row is optional. <code class="font-mono">artifact</code> may be left blank for
          version-level blocks. <code class="font-mono">reason</code> is used only for block
          actions.
        </p>
      </CardContent>
    </Card>

    <!-- Action + input -->
    <Card>
      <CardHeader class="pb-3">
        <CardTitle class="text-base"> Configure import </CardTitle>
      </CardHeader>
      <CardContent class="space-y-4">
        <!-- Action selector -->
        <div class="flex gap-2">
          <Button
            :variant="action === 'block' ? 'default' : 'outline'"
            size="sm"
            @click="
              action = 'block';
              parsedRows = [];
              result = null;
            "
          >
            Block
          </Button>
          <Button
            :variant="action === 'unblock' ? 'default' : 'outline'"
            size="sm"
            @click="
              action = 'unblock';
              parsedRows = [];
              result = null;
            "
          >
            Unblock
          </Button>
        </div>

        <!-- Default reason (block only) -->
        <div v-if="action === 'block'" class="space-y-1 max-w-md">
          <Label for="default-reason"
            >Default reason
            <span class="text-muted-foreground">(used when the CSV row has no reason)</span></Label
          >
          <Input
            id="default-reason"
            v-model="defaultReason"
            placeholder="CVE-2025-XXXX or policy violation"
          />
        </div>

        <!-- CSV textarea -->
        <div class="space-y-1">
          <Label for="csv-input">Paste CSV</Label>
          <textarea
            id="csv-input"
            v-model="csvText"
            rows="8"
            placeholder="registry,name,version,artifact,reason&#10;npm,lodash,4.17.21,,CVE-2021-23337"
            class="flex w-full rounded-sm border border-input bg-transparent px-3 py-2 text-sm font-mono shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring resize-y"
          />
        </div>

        <!-- File upload -->
        <div class="flex items-center gap-3">
          <label
            class="flex items-center gap-2 px-3 py-1.5 rounded-sm border border-input text-sm cursor-pointer hover:bg-accent transition-colors"
          >
            <Upload class="h-3.5 w-3.5" />
            Upload .csv file
            <input type="file" accept=".csv,text/csv" class="sr-only" @change="handleFileUpload" />
          </label>
          <span class="text-xs text-muted-foreground">or paste above</span>
        </div>

        <p v-if="parseError" class="text-xs text-destructive">
          {{ parseError }}
        </p>

        <div class="flex gap-2">
          <Button variant="outline" @click="parseCSV"> Preview rows </Button>
          <Button variant="ghost" size="sm" @click="reset"> Reset </Button>
        </div>
      </CardContent>
    </Card>

    <!-- Preview table -->
    <Card v-if="parsedRows.length > 0">
      <CardHeader class="pb-3">
        <div class="flex items-center justify-between">
          <CardTitle class="text-base">
            Preview
            <span class="font-normal text-muted-foreground ml-1 text-sm">
              {{ validRows.length }} valid, {{ invalidRows.length }} invalid
            </span>
          </CardTitle>
          <Button :disabled="validRows.length === 0 || submitting" @click="submit">
            {{
              submitting
                ? "Processing…"
                : `${action === "block" ? "Block" : "Unblock"} ${validRows.length} package${validRows.length !== 1 ? "s" : ""}`
            }}
          </Button>
        </div>
      </CardHeader>
      <CardContent class="p-0">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Registry</TableHead>
              <TableHead>Name</TableHead>
              <TableHead>Version</TableHead>
              <TableHead>Artifact</TableHead>
              <TableHead v-if="action === 'block'"> Reason </TableHead>
              <TableHead>Status</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow
              v-for="(row, i) in parsedRows"
              :key="i"
              :class="row.error ? 'bg-destructive/5' : ''"
            >
              <TableCell class="font-mono text-xs">
                {{ row.registry || "—" }}
              </TableCell>
              <TableCell class="font-mono text-xs">
                {{ row.name || "—" }}
              </TableCell>
              <TableCell class="font-mono text-xs">
                {{ row.version || "—" }}
              </TableCell>
              <TableCell class="font-mono text-xs text-muted-foreground">
                {{ row.artifact || "—" }}
              </TableCell>
              <TableCell v-if="action === 'block'" class="text-xs">
                {{ row.reason || defaultReason || "—" }}
              </TableCell>
              <TableCell>
                <Badge v-if="row.error" variant="destructive" class="text-xs">
                  {{ row.error }}
                </Badge>
                <Badge v-else variant="secondary" class="text-xs"> OK </Badge>
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
      </CardContent>
    </Card>

    <!-- Results -->
    <Card v-if="result">
      <CardHeader class="pb-3">
        <CardTitle class="text-base flex items-center gap-2">
          <CheckCircle2 class="h-4 w-4 text-primary" />
          Done — {{ result.succeeded_count }} succeeded, {{ result.failed_count }} failed
        </CardTitle>
      </CardHeader>
      <CardContent v-if="result.failures.length > 0" class="p-0">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Registry</TableHead>
              <TableHead>Name</TableHead>
              <TableHead>Version</TableHead>
              <TableHead>Error</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            <TableRow v-for="(f, i) in result.failures" :key="i" class="bg-destructive/5">
              <TableCell class="font-mono text-xs">
                {{ f.registry }}
              </TableCell>
              <TableCell class="font-mono text-xs">
                {{ f.name }}
              </TableCell>
              <TableCell class="font-mono text-xs">
                {{ f.version }}
              </TableCell>
              <TableCell class="text-xs text-destructive flex items-center gap-1">
                <XCircle class="h-3 w-3 shrink-0" />{{ f.error }}
              </TableCell>
            </TableRow>
          </TableBody>
        </Table>
      </CardContent>
    </Card>

    <p v-if="submitError" class="text-sm text-destructive">
      {{ submitError }}
    </p>
  </div>
</template>
