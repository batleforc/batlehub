<script setup lang="ts">
import { ref, computed, watch } from "vue";
import { useAuth } from "@/composables/useAuth";
import { useApi } from "@/composables/useApi";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card, CardHeader, CardTitle, CardContent, CardDescription,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import Select from "@/components/ui/select/Select.vue";
import {
  Table, TableHeader, TableBody, TableRow, TableHead, TableCell,
} from "@/components/ui/table";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import CodeBlock from "@/components/ui/code-block/CodeBlock.vue";

const { token, identity } = useAuth();
const API_BASE = import.meta.env.VITE_API_BASE_URL ?? "";

type Visibility = "public" | "internal" | "team";

interface RegistryInfo {
  name: string;
  type: string;
  mode: string;
}

interface TeamNamespaceDto {
  registry: string;
  prefix: string;
  group_id: string;
  claimed_by: string | null;
}

interface NamespacePackageDto {
  name: string;
  version: string;
  visibility: Visibility;
  published_by: string;
  published_at: string;
  yanked: boolean;
}

const groups = computed(() => identity.value?.groups ?? []);
const hasGroups = computed(() => groups.value.length > 0);

// ── Registries ────────────────────────────────────────────────────────────────

const { data: registriesData } = useApi<RegistryInfo[]>(
  () =>
    fetch(`${API_BASE}/api/v1/registries`, {
      headers: token.value ? { Authorization: `Bearer ${token.value}` } : {},
    }).then(async (r) => {
      if (!r.ok) throw new Error(await r.text());
      return { data: await r.json() };
    }) as Promise<{ data?: unknown; error?: unknown }>,
  [token],
);

const uploadableRegistries = computed(() =>
  (registriesData.value ?? []).filter(
    (r) => r.mode === "local" || r.mode === "hybrid",
  ),
);

// ── My Namespaces ─────────────────────────────────────────────────────────────

const {
  data: myNamespaces,
  error: namespacesError,
  loading: namespacesLoading,
} = useApi<TeamNamespaceDto[]>(
  () => {
    if (!token.value) return Promise.resolve({ data: [] }) as Promise<{ data?: unknown; error?: unknown }>;
    return fetch(`${API_BASE}/api/v1/me/namespaces`, {
      headers: { Authorization: `Bearer ${token.value}` },
    }).then(async (r) => {
      if (!r.ok) throw new Error(await r.text());
      return { data: await r.json() };
    }) as Promise<{ data?: unknown; error?: unknown }>;
  },
  [token],
);

// ── Packages section ──────────────────────────────────────────────────────────

const selectedNs = ref<TeamNamespaceDto | null>(null);
const pkgPage = ref(0);
const pkgPageSize = 50;
const pkgsTrigger = ref(0);

const {
  data: pkgsData,
  error: pkgsError,
  loading: pkgsLoading,
} = useApi<NamespacePackageDto[]>(
  () => {
    if (!selectedNs.value || !token.value) {
      return Promise.resolve({ data: undefined }) as Promise<{ data?: unknown; error?: unknown }>;
    }
    void pkgsTrigger.value;
    const { registry, prefix } = selectedNs.value;
    const url = `${API_BASE}/api/v1/me/namespaces/${encodeURIComponent(registry)}/${encodeURIComponent(prefix)}/packages?page=${pkgPage.value}&per_page=${pkgPageSize}`;
    return fetch(url, {
      headers: { Authorization: `Bearer ${token.value}` },
    }).then(async (r) => {
      if (!r.ok) throw new Error(await r.text());
      return { data: await r.json() };
    }) as Promise<{ data?: unknown; error?: unknown }>;
  },
  [token, selectedNs, pkgsTrigger],
);

function selectNamespace(ns: TeamNamespaceDto) {
  selectedNs.value = ns;
  pkgPage.value = 0;
}

function prevPage() {
  if (pkgPage.value > 0) { pkgPage.value--; pkgsTrigger.value++; }
}
function nextPage() {
  if ((pkgsData.value?.length ?? 0) >= pkgPageSize) { pkgPage.value++; pkgsTrigger.value++; }
}

// ── Inline visibility editing ─────────────────────────────────────────────────

const editingVisibility = ref<Record<string, Visibility>>({});
const visibilitySaving = ref<Record<string, boolean>>({});
const visibilityError = ref<Record<string, string>>({});

function pkgKey(pkg: NamespacePackageDto) {
  return `${selectedNs.value?.registry}|${pkg.name}|${pkg.version}`;
}

function startEdit(pkg: NamespacePackageDto) {
  const k = pkgKey(pkg);
  editingVisibility.value = { ...editingVisibility.value, [k]: pkg.visibility };
}

function cancelEdit(pkg: NamespacePackageDto) {
  const k = pkgKey(pkg);
  const copy = { ...editingVisibility.value };
  delete copy[k];
  editingVisibility.value = copy;
}

async function saveVisibility(pkg: NamespacePackageDto) {
  if (!selectedNs.value || !token.value) return;
  const k = pkgKey(pkg);
  const vis = editingVisibility.value[k];
  visibilitySaving.value = { ...visibilitySaving.value, [k]: true };
  visibilityError.value = { ...visibilityError.value, [k]: "" };
  try {
    const r = await fetch(
      `${API_BASE}/api/v1/admin/registries/${encodeURIComponent(selectedNs.value.registry)}/packages/${encodeURIComponent(pkg.name)}/visibility`,
      {
        method: "PUT",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${token.value}`,
        },
        body: JSON.stringify({ visibility: vis }),
      },
    );
    if (!r.ok) throw new Error(await r.text());
    pkg.visibility = vis;
    cancelEdit(pkg);
    pkgsTrigger.value++;
  } catch (e) {
    visibilityError.value = { ...visibilityError.value, [k]: e instanceof Error ? e.message : "Error" };
  } finally {
    visibilitySaving.value = { ...visibilitySaving.value, [k]: false };
  }
}

const visibilityOptions = [
  { value: "public",   label: "Public" },
  { value: "internal", label: "Internal" },
  { value: "team",     label: "Team" },
];

// ── Upload ────────────────────────────────────────────────────────────────────

const selectedUploadRegistry = ref("");
watch(uploadableRegistries, (list) => {
  if (list.length > 0 && !selectedUploadRegistry.value) {
    selectedUploadRegistry.value = list[0].name;
  }
});

const uploadRegistryOptions = computed(() =>
  uploadableRegistries.value.map((r) => ({ value: r.name, label: `${r.name} (${r.type})` })),
);

const uploadRegistryType = computed(
  () => uploadableRegistries.value.find((r) => r.name === selectedUploadRegistry.value)?.type ?? "",
);

const binaryUploadTypes = ["rubygems", "composer", "openvsx", "vscode-marketplace", "goproxy"];
const isBinaryUpload = computed(() => binaryUploadTypes.includes(uploadRegistryType.value));

// Fields for upload
const uploadFile = ref<File | null>(null);
const uploadExtId = ref(""); // publisher.name for openvsx
const uploadVersion = ref(""); // version for openvsx / goproxy
const uploadModule = ref(""); // module path for goproxy
const uploadLoading = ref(false);
const uploadError = ref<string | null>(null);
const uploadSuccess = ref(false);

function onFileChange(e: Event) {
  const target = e.target as HTMLInputElement;
  uploadFile.value = target.files?.[0] ?? null;
}

function acceptForType(t: string) {
  if (t === "rubygems") return ".gem";
  if (t === "composer") return ".zip";
  if (t === "openvsx" || t === "vscode-marketplace") return ".vsix";
  if (t === "goproxy") return ".zip";
  return "*";
}

async function doUpload() {
  if (!uploadFile.value || !selectedUploadRegistry.value || !token.value) return;
  uploadLoading.value = true;
  uploadError.value = null;
  uploadSuccess.value = false;
  try {
    const reg = encodeURIComponent(selectedUploadRegistry.value);
    let url = "";
    let method = "POST";
    if (uploadRegistryType.value === "rubygems") {
      url = `${API_BASE}/proxy/${reg}/api/v1/gems`;
    } else if (uploadRegistryType.value === "composer") {
      url = `${API_BASE}/proxy/${reg}/api/upload`;
    } else if (uploadRegistryType.value === "openvsx" || uploadRegistryType.value === "vscode-marketplace") {
      if (!uploadExtId.value.trim() || !uploadVersion.value.trim()) {
        throw new Error("Extension ID and version are required for VS Code extensions");
      }
      url = `${API_BASE}/proxy/${reg}/${encodeURIComponent(uploadExtId.value.trim())}/${encodeURIComponent(uploadVersion.value.trim())}/vsix`;
      method = "PUT";
    } else if (uploadRegistryType.value === "goproxy") {
      if (!uploadModule.value.trim() || !uploadVersion.value.trim()) {
        throw new Error("Module path and version are required for Go modules");
      }
      url = `${API_BASE}/proxy/${reg}/${encodeURIComponent(uploadModule.value.trim())}/@v/${encodeURIComponent(uploadVersion.value.trim())}.zip`;
      method = "PUT";
    }

    const body = await uploadFile.value.arrayBuffer();
    const r = await fetch(url, {
      method,
      headers: {
        "Content-Type": "application/octet-stream",
        Authorization: `Bearer ${token.value}`,
      },
      body,
    });
    if (!r.ok) throw new Error(await r.text());
    uploadSuccess.value = true;
    uploadFile.value = null;
    uploadExtId.value = "";
    uploadVersion.value = "";
    uploadModule.value = "";
    pkgsTrigger.value++;
    setTimeout(() => { uploadSuccess.value = false; }, 4000);
  } catch (e) {
    uploadError.value = e instanceof Error ? e.message : "Unknown error";
  } finally {
    uploadLoading.value = false;
  }
}

// ── CLI snippets ──────────────────────────────────────────────────────────────

const cliRegistry = computed(() => selectedUploadRegistry.value || "<registry>");

const cliSnippets: Record<string, string> = {
  npm: `# Configure npm to use this registry
npm set registry ${window.location.origin}/proxy/${cliRegistry.value}
npm publish`,
  cargo: `# In your .cargo/config.toml:
[registries.${cliRegistry.value}]
index = "sparse+${window.location.origin}/proxy/${cliRegistry.value}/registry/"

# Then publish:
cargo publish --registry ${cliRegistry.value}`,
  maven: `<!-- In your settings.xml: -->
<server>
  <id>${cliRegistry.value}</id>
  <username>your-user</username>
  <password>your-token</password>
</server>

<!-- In your pom.xml: -->
<distributionManagement>
  <repository>
    <id>${cliRegistry.value}</id>
    <url>${window.location.origin}/proxy/${cliRegistry.value}/maven2</url>
  </repository>
</distributionManagement>

mvn deploy`,
  terraform: `# In your Terraform config:
terraform {
  required_providers {
    <provider> = {
      source = "${window.location.hostname}/${cliRegistry.value}/<namespace>/<provider>"
    }
  }
}`,
};

const currentCliSnippet = computed(
  () => cliSnippets[uploadRegistryType.value] ?? "# No CLI instructions available for this registry type.",
);

// ── Helpers ───────────────────────────────────────────────────────────────────

function visibilityVariant(v: Visibility) {
  if (v === "public") return "default";
  if (v === "internal") return "secondary";
  return "outline";
}

function formatDate(iso: string) {
  return new Date(iso).toLocaleDateString(undefined, { dateStyle: "medium" });
}
</script>

<template>
  <div class="space-y-6 max-w-4xl">
    <!-- Header -->
    <div>
      <h1 class="text-2xl font-semibold">
        Team Namespace
      </h1>
      <p class="text-sm text-muted-foreground mt-0.5">
        View and manage the packages and namespaces owned by your groups.
      </p>
    </div>

    <!-- No groups -->
    <Card v-if="!hasGroups">
      <CardContent class="pt-6">
        <p class="text-sm text-muted-foreground">
          You are not a member of any groups. Contact your administrator to be added to a team namespace.
        </p>
      </CardContent>
    </Card>

    <template v-else>
      <!-- Groups -->
      <Card>
        <CardHeader>
          <CardTitle class="text-base">
            Your groups
          </CardTitle>
        </CardHeader>
        <CardContent>
          <div class="flex flex-wrap gap-2">
            <Badge
              v-for="g in groups"
              :key="g"
              variant="secondary"
              class="font-mono text-xs"
            >
              {{ g.replaceAll(' ', '') }}
            </Badge>
          </div>
        </CardContent>
      </Card>

      <!-- My Namespaces -->
      <Card>
        <CardHeader>
          <CardTitle class="text-base">
            My namespaces
          </CardTitle>
          <CardDescription>
            Namespace prefixes your groups own. Click a row to browse its packages.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <p
            v-if="namespacesLoading"
            class="text-sm text-muted-foreground"
          >
            Loading…
          </p>
          <p
            v-else-if="namespacesError"
            class="text-sm text-destructive"
          >
            {{ namespacesError }}
          </p>
          <p
            v-else-if="!myNamespaces?.length"
            class="text-sm text-muted-foreground"
          >
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
                :class="selectedNs?.registry === ns.registry && selectedNs?.prefix === ns.prefix
                  ? 'bg-muted/60'
                  : 'hover:bg-muted/40'"
                @click="selectNamespace(ns)"
              >
                <TableCell class="font-mono text-xs">
                  {{ ns.registry }}
                </TableCell>
                <TableCell class="font-mono text-xs">
                  {{ ns.prefix }}
                </TableCell>
                <TableCell class="font-mono text-xs">
                  {{ ns.group_id.replaceAll(' ', '') }}
                </TableCell>
              </TableRow>
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      <!-- Packages -->
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
            {{ selectedNs ? "Published versions under the selected namespace." : "Select a namespace row above to browse its packages." }}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <p
            v-if="pkgsLoading"
            class="text-sm text-muted-foreground"
          >
            Loading…
          </p>
          <p
            v-else-if="pkgsError"
            class="text-sm text-destructive"
          >
            {{ pkgsError }}
          </p>
          <p
            v-else-if="selectedNs && !pkgsData?.length"
            class="text-sm text-muted-foreground"
          >
            No published packages found under this namespace.
          </p>
          <template v-else-if="pkgsData?.length">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Package</TableHead>
                  <TableHead>Version</TableHead>
                  <TableHead>Visibility</TableHead>
                  <TableHead>Published by</TableHead>
                  <TableHead>Date</TableHead>
                  <TableHead />
                </TableRow>
              </TableHeader>
              <TableBody>
                <TableRow
                  v-for="pkg in pkgsData"
                  :key="`${pkg.name}@${pkg.version}`"
                  :class="pkg.yanked ? 'opacity-50' : ''"
                >
                  <TableCell class="font-mono text-xs">
                    {{ pkg.name }}
                  </TableCell>
                  <TableCell class="font-mono text-xs">
                    {{ pkg.version }}
                    <span
                      v-if="pkg.yanked"
                      class="ml-1 text-destructive"
                    >(yanked)</span>
                  </TableCell>
                  <TableCell>
                    <template v-if="editingVisibility[pkgKey(pkg)] !== undefined">
                      <div class="flex items-center gap-1">
                        <Select
                          v-model="editingVisibility[pkgKey(pkg)]"
                          :options="visibilityOptions"
                          class="w-32 text-xs"
                        />
                        <Button
                          size="sm"
                          variant="default"
                          :disabled="visibilitySaving[pkgKey(pkg)]"
                          class="text-xs h-7 px-2"
                          @click="saveVisibility(pkg)"
                        >
                          {{ visibilitySaving[pkgKey(pkg)] ? "…" : "Save" }}
                        </Button>
                        <Button
                          size="sm"
                          variant="ghost"
                          class="text-xs h-7 px-2"
                          @click="cancelEdit(pkg)"
                        >
                          Cancel
                        </Button>
                      </div>
                      <p
                        v-if="visibilityError[pkgKey(pkg)]"
                        class="text-xs text-destructive mt-0.5"
                      >
                        {{ visibilityError[pkgKey(pkg)] }}
                      </p>
                    </template>
                    <Badge
                      v-else
                      :variant="visibilityVariant(pkg.visibility)"
                      class="capitalize text-xs cursor-pointer"
                      :class="pkg.visibility === 'team' ? 'border-blue-500 text-blue-600' : ''"
                      @click="startEdit(pkg)"
                    >
                      {{ pkg.visibility }}
                    </Badge>
                  </TableCell>
                  <TableCell class="text-xs">
                    {{ pkg.published_by }}
                  </TableCell>
                  <TableCell class="text-xs">
                    {{ formatDate(pkg.published_at) }}
                  </TableCell>
                  <TableCell>
                    <Button
                      v-if="editingVisibility[pkgKey(pkg)] === undefined"
                      size="sm"
                      variant="ghost"
                      class="text-xs h-7 px-2"
                      @click="startEdit(pkg)"
                    >
                      Edit visibility
                    </Button>
                  </TableCell>
                </TableRow>
              </TableBody>
            </Table>
            <!-- Pagination -->
            <div class="flex items-center justify-between mt-3">
              <Button
                variant="outline"
                size="sm"
                :disabled="pkgPage === 0"
                @click="prevPage"
              >
                Previous
              </Button>
              <span class="text-xs text-muted-foreground">Page {{ pkgPage + 1 }}</span>
              <Button
                variant="outline"
                size="sm"
                :disabled="(pkgsData?.length ?? 0) < pkgPageSize"
                @click="nextPage"
              >
                Next
              </Button>
            </div>
          </template>
        </CardContent>
      </Card>

      <!-- Upload -->
      <Card>
        <CardHeader>
          <CardTitle class="text-base">
            Upload package
          </CardTitle>
          <CardDescription>
            Publish a new package to one of your registries.
          </CardDescription>
        </CardHeader>
        <CardContent class="space-y-4">
          <div
            v-if="!uploadableRegistries.length"
            class="text-sm text-muted-foreground"
          >
            No registries in Local or Hybrid mode are configured.
          </div>
          <template v-else>
            <!-- Registry picker -->
            <div class="space-y-1.5 w-60">
              <Label>Registry</Label>
              <Select
                v-model="selectedUploadRegistry"
                :options="uploadRegistryOptions"
                placeholder="Select registry…"
              />
            </div>

            <Tabs
              :default-value="isBinaryUpload ? 'upload' : 'cli'"
              class="w-full"
            >
              <TabsList>
                <TabsTrigger
                  v-if="isBinaryUpload"
                  value="upload"
                >
                  File upload
                </TabsTrigger>
                <TabsTrigger value="cli">
                  CLI instructions
                </TabsTrigger>
              </TabsList>

              <!-- Direct file upload -->
              <TabsContent
                v-if="isBinaryUpload"
                value="upload"
                class="space-y-4 pt-4"
              >
                <!-- Extra fields for openvsx / goproxy -->
                <template v-if="uploadRegistryType === 'openvsx' || uploadRegistryType === 'vscode-marketplace'">
                  <div class="grid grid-cols-2 gap-3">
                    <div class="space-y-1.5">
                      <Label>Extension ID <span class="text-muted-foreground text-xs">(publisher.name)</span></Label>
                      <Input
                        v-model="uploadExtId"
                        placeholder="ms-python.python"
                        class="font-mono text-sm"
                      />
                    </div>
                    <div class="space-y-1.5">
                      <Label>Version</Label>
                      <Input
                        v-model="uploadVersion"
                        placeholder="1.0.0"
                        class="font-mono text-sm"
                      />
                    </div>
                  </div>
                </template>
                <template v-else-if="uploadRegistryType === 'goproxy'">
                  <div class="grid grid-cols-2 gap-3">
                    <div class="space-y-1.5">
                      <Label>Module path</Label>
                      <Input
                        v-model="uploadModule"
                        placeholder="github.com/org/repo"
                        class="font-mono text-sm"
                      />
                    </div>
                    <div class="space-y-1.5">
                      <Label>Version</Label>
                      <Input
                        v-model="uploadVersion"
                        placeholder="v1.0.0"
                        class="font-mono text-sm"
                      />
                    </div>
                  </div>
                </template>

                <!-- File input -->
                <div class="space-y-1.5">
                  <Label>
                    File
                    <span class="text-muted-foreground text-xs ml-1">{{ acceptForType(uploadRegistryType) }}</span>
                  </Label>
                  <input
                    type="file"
                    :accept="acceptForType(uploadRegistryType)"
                    class="block text-sm text-muted-foreground file:mr-3 file:py-1.5 file:px-3 file:rounded-md file:border-0 file:text-xs file:font-medium file:bg-secondary file:text-secondary-foreground hover:file:bg-secondary/80 cursor-pointer"
                    @change="onFileChange"
                  >
                </div>

                <div class="flex items-center gap-3">
                  <Button
                    :disabled="!uploadFile || uploadLoading"
                    @click="doUpload"
                  >
                    {{ uploadLoading ? "Uploading…" : "Upload" }}
                  </Button>
                  <span
                    v-if="uploadSuccess"
                    class="text-sm text-green-600"
                  >Published successfully.</span>
                  <span
                    v-if="uploadError"
                    class="text-sm text-destructive"
                  >{{ uploadError }}</span>
                </div>
              </TabsContent>

              <!-- CLI instructions -->
              <TabsContent
                value="cli"
                class="pt-4"
              >
                <CodeBlock
                  :code="currentCliSnippet"
                  lang="bash"
                />
              </TabsContent>
            </Tabs>
          </template>
        </CardContent>
      </Card>
    </template>
  </div>
</template>
