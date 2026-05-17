<script setup lang="ts">
import { useDark, useToggle } from "@vueuse/core";
import { Sun, Moon } from "@lucide/vue";
import { Button } from "@/components/ui/button";

const isDark = useDark();
const toggle = useToggle(isDark);
</script>

<template>
  <Button variant="ghost" size="icon" @click="toggle()" :title="isDark ? 'Switch to light mode' : 'Switch to dark mode'">
    <Sun v-if="isDark" class="h-4 w-4" />
    <Moon v-else class="h-4 w-4" />
  </Button>
</template>
