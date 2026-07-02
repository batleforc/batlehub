<script setup lang="ts">
import { computed } from "vue";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";
import type { PathFieldDef, RegistryPathTypeDef } from "@/config/registryPathFields";

const props = defineProps<{
  typeDef: RegistryPathTypeDef;
  registries: { name: string; type: string }[];
}>();

const registryName = defineModel<string>("registryName", { required: true });
const values = defineModel<Record<string, string>>("values", { required: true });

const GRID_COLS: Record<number, string> = { 2: "grid-cols-2", 3: "grid-cols-3" };

const rows = computed<PathFieldDef[][]>(() => {
  const result: PathFieldDef[][] = [];
  for (const field of props.typeDef.fields) {
    const last = result[result.length - 1]?.[0];
    if (field.row !== undefined && last?.row === field.row) {
      result[result.length - 1].push(field);
    } else {
      result.push([field]);
    }
  }
  return result;
});
</script>

<template>
  <div class="space-y-4">
    <div class="space-y-1">
      <Label :for="`${typeDef.id}-registry`">Registry name</Label>
      <Input
        :id="`${typeDef.id}-registry`"
        v-model="registryName"
        :list="`pm-${typeDef.id}-list`"
        :placeholder="typeDef.id"
        class="font-mono"
      />
      <datalist :id="`pm-${typeDef.id}-list`">
        <option v-for="r in registries" :key="r.name" :value="r.name" />
      </datalist>
    </div>

    <div
      v-for="(row, i) in rows"
      :key="i"
      :class="row.length > 1 ? ['grid gap-3', GRID_COLS[row.length]] : ''"
    >
      <div v-for="field in row" :key="field.key" class="space-y-1">
        <Label :for="`${typeDef.id}-${field.key}`">
          {{ field.label }}
          <span v-if="field.suffix" class="text-muted-foreground">{{ field.suffix }}</span>
        </Label>
        <Input
          :id="`${typeDef.id}-${field.key}`"
          v-model="values[field.key]"
          :placeholder="field.placeholder"
          :class="cn(field.mono && 'font-mono')"
        />
      </div>
    </div>

    <p v-if="typeDef.note" class="text-xs text-muted-foreground" v-html="typeDef.note" />
  </div>
</template>
