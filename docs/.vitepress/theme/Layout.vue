<script setup lang="ts">
import DefaultTheme from "vitepress/theme";
import RfcStatus from "./RfcStatus.vue";

// The default layout with one addition: the RFC status banner, in the slot the
// default theme provides above the document title. Extending by slot rather
// than by replacing a component keeps the theme's own accessibility work and
// its upgrade path (RFC 0005 Q1 — bridge, don't override).
const { Layout } = DefaultTheme;
</script>

<template>
  <Layout>
    <template #doc-before>
      <RfcStatus />
    </template>
  </Layout>
</template>
