<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { TOOLS_TABS } from "@/config/navigation";
import HubLayout from "./HubLayout.vue";

const { t } = useI18n();

/* Diagnostics: used rarely, and mainly when something has already gone wrong.
   Demoted out of the primary nav and linked from the errors that motivate
   them — a denied request is where these earn their place. */
</script>

<template>
  <HubLayout :title="t('tools.title')" :description="t('tools.description')" :tabs="TOOLS_TABS" />
</template>
