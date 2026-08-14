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

    <!-- Full width, matching the masthead and the footer.

         This was `container mx-auto`, capped at 1400px, while both ruled bars
         run edge to edge — so on anything wider the frame's rails reached the
         window and the sheet inside them stopped ~260px short, which reads as a
         page that failed to load its own layout rather than as a measure. The
         design proof is full-bleed throughout (`.specimen`, `.catalog` and
         `.masthead` all take padding, none takes a max-width), and the edge
         padding here is the same 16 → 24 step the bars use so all three line
         up on one margin.

         Pages that want a reading measure still set their own: the specimen
         caption caps at `72ch`, `EmptyState` and the panels at `64ch`. Those
         are per-surface decisions, which is where they belong — a global
         container was applying one answer to a catalog table and a paragraph
         of prose alike. -->
    <main id="main" tabindex="-1" class="flex-1 px-4 py-6 md:px-6">
      <RouterView />
    </main>

    <AppFooter />
  </div>
</template>
