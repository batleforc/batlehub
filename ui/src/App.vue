<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { ref } from "vue";
import { RouterView } from "vue-router";
import AppHeader from "@/components/app/AppHeader.vue";
import AppFooter from "@/components/app/AppFooter.vue";

const { t } = useI18n();

const mobileOpen = ref(false);
</script>

<template>
  <div class="min-h-screen flex flex-col bg-background">
    <!-- First tab stop on every page: skipping the header is the difference
         between reaching the content in one key and in twenty. -->
    <a
      href="#main"
      class="sr-only focus:not-sr-only focus:absolute focus:left-2 focus:top-2 focus:z-50 focus:bg-primary focus:px-4 focus:py-3 focus:font-mono focus:text-sm focus:text-primary-foreground"
    >
      {{ t("a11y.skipToContent") }}
    </a>

    <AppHeader v-model:mobile-open="mobileOpen" />

    <main id="main" tabindex="-1" class="container mx-auto px-4 py-6 flex-1">
      <RouterView />
    </main>

    <AppFooter />
  </div>
</template>
