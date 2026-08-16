<script setup lang="ts">
import { useI18n } from "vue-i18n";
/**
 * The read-only configuration screen.
 *
 * When the config is mounted read-only — a Kubernetes ConfigMap, most often —
 * editing it here is not merely disabled, it is *not a thing that exists*. The
 * previous shape was a yellow banner above a textarea that still looked like a
 * working editor, with Validate and Apply beneath it: an operator could type a
 * change, watch it validate, and only then discover nothing would be saved.
 *
 * So this is a different screen, not a disabled one. It shows what is running
 * and says where to change it (RFC 0003 §6.4).
 */
import { CodeBlock } from "@/components/ui/code-block";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CopyButton } from "@/components/ui/copy-button";

const { t } = useI18n();

defineProps<{ content: string; error?: string | null }>();
</script>

<template>
  <Card>
    <CardHeader class="flex-row items-center justify-between gap-4 space-y-0">
      <CardTitle>{{ t("config.readOnlyTitle") }}</CardTitle>
      <CopyButton :text="content" size="sm" variant="outline" :label="t('config.copyToml')" />
    </CardHeader>
    <CardContent class="space-y-3">
      <p class="text-sm text-muted-foreground">
        {{ t("config.readOnlyBody") }}
      </p>
      <p v-if="error" class="text-sm text-destructive">{{ error }}</p>
      <CodeBlock v-else :code="content" lang="toml" />
    </CardContent>
  </Card>
</template>
