<script setup lang="ts">
import { ref } from "vue";
import { RouterView } from "vue-router";
import AppHeader from "@/components/app/AppHeader.vue";
import AppFooter from "@/components/app/AppFooter.vue";

const mobileOpen = ref(false);
</script>

<template>
  <div class="min-h-screen flex flex-col bg-background">
    <AppHeader v-model:mobile-open="mobileOpen" />

    <main class="container mx-auto px-4 py-6 flex-1">
      <RouterView />
    </main>

    <AppFooter />
  </div>
</template>
