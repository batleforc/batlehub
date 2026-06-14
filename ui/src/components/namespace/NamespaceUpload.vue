<script setup lang="ts">
import { ref, computed } from "vue";
import type { RegistryInfo } from "@/client/types.gen";
import { useAuthFetch } from "@/composables/useAuthFetch";
import { API_BASE_URL } from "@/config";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import Select from "@/components/ui/select/Select.vue";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import CodeBlock from "@/components/ui/code-block/CodeBlock.vue";

const props = defineProps<{ registries: RegistryInfo[] }>();

const { authFetch } = useAuthFetch();

const uploadableRegistries = computed(() =>
  props.registries.filter((r) => r.mode === "local" || r.mode === "hybrid"),
);

const selectedRegistry = ref("");
const registryOptions = computed(() =>
  uploadableRegistries.value.map((r) => ({ value: r.name, label: r.name })),
);
const registryType = computed(
  () => props.registries.find((r) => r.name === selectedRegistry.value)?.type ?? "",
);

const BINARY_TYPES = new Set([
  "rubygems",
  "composer",
  "openvsx",
  "vscode-marketplace",
  "goproxy",
  "deb",
  "rpm",
]);
const isBinaryUpload = computed(() => BINARY_TYPES.has(registryType.value));

const uploadFile = ref<File | null>(null);
const uploadExtId = ref("");
const uploadVersion = ref("");
const uploadModule = ref("");
const uploadDistribution = ref("");
const uploadComponent = ref("main");
const loading = ref(false);
const error = ref<string | null>(null);
const success = ref(false);

function onFileChange(e: Event) {
  uploadFile.value = (e.target as HTMLInputElement).files?.[0] ?? null;
}

function acceptFor(t: string) {
  if (t === "rubygems") return ".gem";
  if (t === "composer") return ".zip";
  if (t === "openvsx" || t === "vscode-marketplace") return ".vsix";
  if (t === "goproxy") return ".zip";
  if (t === "deb") return ".deb";
  if (t === "rpm") return ".rpm";
  return "*";
}

async function doUpload() {
  if (!uploadFile.value || !selectedRegistry.value) return;
  loading.value = true;
  error.value = null;
  success.value = false;
  try {
    const reg = encodeURIComponent(selectedRegistry.value);
    let url = "";
    let method = "POST";
    if (registryType.value === "rubygems") {
      url = `${API_BASE_URL}/proxy/${reg}/api/v1/gems`;
    } else if (registryType.value === "composer") {
      url = `${API_BASE_URL}/proxy/${reg}/api/upload`;
    } else if (registryType.value === "openvsx" || registryType.value === "vscode-marketplace") {
      if (!uploadExtId.value.trim() || !uploadVersion.value.trim())
        throw new Error("Extension ID and version are required");
      url = `${API_BASE_URL}/proxy/${reg}/${encodeURIComponent(uploadExtId.value.trim())}/${encodeURIComponent(uploadVersion.value.trim())}/vsix`;
      method = "PUT";
    } else if (registryType.value === "goproxy") {
      if (!uploadModule.value.trim() || !uploadVersion.value.trim())
        throw new Error("Module path and version are required");
      url = `${API_BASE_URL}/proxy/${reg}/${encodeURIComponent(uploadModule.value.trim())}/@v/${encodeURIComponent(uploadVersion.value.trim())}.zip`;
      method = "PUT";
    } else if (registryType.value === "deb") {
      if (!uploadDistribution.value.trim() || !uploadComponent.value.trim())
        throw new Error("Distribution and component are required");
      url = `${API_BASE_URL}/proxy/${reg}/deb/pool/${encodeURIComponent(uploadDistribution.value.trim())}/${encodeURIComponent(uploadComponent.value.trim())}/upload`;
      method = "PUT";
    } else if (registryType.value === "rpm") {
      url = `${API_BASE_URL}/proxy/${reg}/rpm/upload`;
      method = "PUT";
    }
    const body = await uploadFile.value.arrayBuffer();
    const r = await authFetch(url, {
      method,
      headers: { "Content-Type": "application/octet-stream" },
      body,
    });
    if (!r.ok) throw new Error(await r.text());
    success.value = true;
    uploadFile.value = null;
    uploadExtId.value = "";
    uploadVersion.value = "";
    uploadModule.value = "";
    uploadDistribution.value = "";
    setTimeout(() => {
      success.value = false;
    }, 4000);
  } catch (e_) {
    error.value = e_ instanceof Error ? e_.message : "Unknown error";
  } finally {
    loading.value = false;
  }
}

const cliRegistryName = computed(() => selectedRegistry.value || "<registry>");
const cliSnippets: Record<string, string> = {
  npm: `npm set registry ${globalThis.location.origin}/proxy/${cliRegistryName.value}\nnpm publish`,
  cargo: `# .cargo/config.toml:\n[registries.${cliRegistryName.value}]\nindex = "sparse+${globalThis.location.origin}/proxy/${cliRegistryName.value}/registry/"\n\ncargo publish --registry ${cliRegistryName.value}`,
  maven: `<!-- settings.xml -->\n<server>\n  <id>${cliRegistryName.value}</id>\n  <username>your-user</username>\n  <password>your-token</password>\n</server>\n\n<!-- pom.xml -->\n<distributionManagement>\n  <repository>\n    <id>${cliRegistryName.value}</id>\n    <url>${globalThis.location.origin}/proxy/${cliRegistryName.value}/maven2</url>\n  </repository>\n</distributionManagement>\n\nmvn deploy`,
  terraform: `terraform {\n  required_providers {\n    <provider> = {\n      source = "${globalThis.location.hostname}/${cliRegistryName.value}/<namespace>/<provider>"\n    }\n  }\n}`,
  deb: `# Publish a .deb to pool/{distribution}/{component}\ncurl -X PUT \\\n  -H "Authorization: Bearer <your-token>" \\\n  --data-binary @hello_1.0_amd64.deb \\\n  ${globalThis.location.origin}/proxy/${cliRegistryName.value}/deb/pool/stable/main/upload`,
  rpm: `# Publish an .rpm\ncurl -X PUT \\\n  -H "Authorization: Bearer <your-token>" \\\n  --data-binary @hello-1.0-1.x86_64.rpm \\\n  ${globalThis.location.origin}/proxy/${cliRegistryName.value}/rpm/upload`,
};
const currentSnippet = computed(
  () =>
    cliSnippets[registryType.value] ?? "# No CLI instructions available for this registry type.",
);
</script>

<template>
  <div v-if="!uploadableRegistries.length" class="text-sm text-muted-foreground">
    No registries in Local or Hybrid mode are configured.
  </div>
  <template v-else>
    <div class="space-y-1.5 w-60">
      <Label for="upload-registry">Registry</Label>
      <Select
        id="upload-registry"
        v-model="selectedRegistry"
        :options="registryOptions"
        placeholder="Select registry…"
      />
    </div>

    <Tabs :default-value="isBinaryUpload ? 'upload' : 'cli'" class="w-full">
      <TabsList>
        <TabsTrigger v-if="isBinaryUpload" value="upload">File upload</TabsTrigger>
        <TabsTrigger value="cli">CLI instructions</TabsTrigger>
      </TabsList>

      <TabsContent v-if="isBinaryUpload" value="upload" class="space-y-4 pt-4">
        <template v-if="registryType === 'openvsx' || registryType === 'vscode-marketplace'">
          <div class="grid grid-cols-2 gap-3">
            <div class="space-y-1.5">
              <Label for="upload-ext-id"
                >Extension ID
                <span class="text-muted-foreground text-xs">(publisher.name)</span></Label
              >
              <Input
                id="upload-ext-id"
                v-model="uploadExtId"
                placeholder="ms-python.python"
                class="font-mono text-sm"
              />
            </div>
            <div class="space-y-1.5">
              <Label for="upload-version-ext">Version</Label>
              <Input
                id="upload-version-ext"
                v-model="uploadVersion"
                placeholder="1.0.0"
                class="font-mono text-sm"
              />
            </div>
          </div>
        </template>
        <template v-else-if="registryType === 'goproxy'">
          <div class="grid grid-cols-2 gap-3">
            <div class="space-y-1.5">
              <Label for="upload-module">Module path</Label>
              <Input
                id="upload-module"
                v-model="uploadModule"
                placeholder="github.com/org/repo"
                class="font-mono text-sm"
              />
            </div>
            <div class="space-y-1.5">
              <Label for="upload-version-module">Version</Label>
              <Input
                id="upload-version-module"
                v-model="uploadVersion"
                placeholder="v1.0.0"
                class="font-mono text-sm"
              />
            </div>
          </div>
        </template>
        <template v-else-if="registryType === 'deb'">
          <div class="grid grid-cols-2 gap-3">
            <div class="space-y-1.5">
              <Label for="upload-distribution"
                >Distribution
                <span class="text-muted-foreground text-xs">(suite)</span></Label
              >
              <Input
                id="upload-distribution"
                v-model="uploadDistribution"
                placeholder="stable"
                class="font-mono text-sm"
              />
            </div>
            <div class="space-y-1.5">
              <Label for="upload-component">Component</Label>
              <Input
                id="upload-component"
                v-model="uploadComponent"
                placeholder="main"
                class="font-mono text-sm"
              />
            </div>
          </div>
        </template>

        <div class="space-y-1.5">
          <Label for="upload-file"
            >File
            <span class="text-muted-foreground text-xs ml-1">{{
              acceptFor(registryType)
            }}</span></Label
          >
          <input
            id="upload-file"
            type="file"
            :accept="acceptFor(registryType)"
            class="block text-sm text-muted-foreground file:mr-3 file:py-1.5 file:px-3 file:rounded-sm file:border-0 file:text-xs file:font-medium file:bg-secondary file:text-secondary-foreground hover:file:bg-secondary/80 cursor-pointer"
            @change="onFileChange"
          />
        </div>

        <div class="flex items-center gap-3">
          <Button :disabled="!uploadFile || loading" @click="doUpload">
            {{ loading ? "Uploading…" : "Upload" }}
          </Button>
          <span v-if="success" class="text-sm text-primary">Published successfully.</span>
          <span v-if="error" class="text-sm text-destructive">{{ error }}</span>
        </div>
      </TabsContent>

      <TabsContent value="cli" class="pt-4">
        <CodeBlock :code="currentSnippet" lang="bash" />
      </TabsContent>
    </Tabs>
  </template>
</template>
