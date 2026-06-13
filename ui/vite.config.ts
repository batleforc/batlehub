import { defineConfig } from "vitest/config";
import vue from "@vitejs/plugin-vue";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  build: {
    outDir: "dist",
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    coverage: {
      provider: "v8",
      reporter: ["text", "lcov", "html"],
      // Explicit allow-list. Add a source file here once it has a
      // corresponding test file; the threshold below applies to this
      // set, not the whole src/ tree.
      include: [
        "src/lib/utils.ts",
        "src/composables/useApi.ts",
        "src/composables/useAuth.ts",
        "src/composables/useAuthFetch.ts",
        "src/composables/useBanner.ts",
        "src/composables/useShiki.ts",
        "src/components/ui/button/Button.vue",
        "src/components/ui/badge/Badge.vue",
        "src/components/ui/alert/Alert.vue",
        "src/components/ui/card/Card.vue",
        "src/components/ui/card/CardHeader.vue",
        "src/components/ui/card/CardTitle.vue",
        "src/components/ui/card/CardDescription.vue",
        "src/components/ui/card/CardContent.vue",
        "src/components/ui/card/CardFooter.vue",
        "src/components/ui/input/Input.vue",
        "src/components/ui/label/Label.vue",
        "src/components/ui/separator/Separator.vue",
        "src/components/ui/switch/Switch.vue",
        "src/components/ui/table/Table.vue",
        "src/components/ui/table/TableHeader.vue",
        "src/components/ui/table/TableHead.vue",
        "src/components/ui/table/TableBody.vue",
        "src/components/ui/table/TableRow.vue",
        "src/components/ui/table/TableCell.vue",
        "src/components/ui/table/TableCaption.vue",
        "src/components/ui/tabs/Tabs.vue",
        "src/components/ui/tabs/TabsList.vue",
        "src/components/ui/tabs/TabsTrigger.vue",
        "src/components/ui/tabs/TabsContent.vue",
        "src/components/ui/select/Select.vue",
        "src/components/ui/dialog/Dialog.vue",
        "src/components/ui/code-block/CodeBlock.vue",
      ],
      thresholds: {
        lines: 80,
        branches: 80,
        functions: 80,
        statements: 80,
      },
    },
  },
});
