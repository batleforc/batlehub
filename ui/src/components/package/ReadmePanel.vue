<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import { FileText } from "@lucide/vue";
import { explorePackageReadme } from "@/client/sdk.gen";
import type { ReadmeResponse } from "@/client/types.gen";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

/**
 * A package version's own account of itself.
 *
 * **The only component in the console that renders server-supplied HTML.** The
 * text is attacker-authored by construction — anyone who can publish to a
 * proxied upstream can write it — and it is displayed on the console's own
 * origin to a session that is frequently an administrator's.
 *
 * Nothing is sanitised here, and that is the design rather than an omission
 * (RFC 0007 §2.6): the boundary is server-side, in `crates/core/src/services/
 * readme/sanitize.rs`, where an allow-list over an `html5ever` parse has unit
 * tests, a fuzz target and `cargo deny` over its dependencies. A markdown
 * renderer plus a DOM sanitiser in this bundle would put that boundary where
 * the fuzz suite cannot reach it, and would have to be reimplemented for the
 * CLI and every other client.
 *
 * `ReadmePanel.test.ts` asserts that no other component grows a `v-html`, so
 * the boundary cannot quietly move.
 */
const props = defineProps<{
  registry: string;
  name: string;
  /** The version the page has selected. `null` asks for the newest that has one. */
  version: string | null;
}>();

const { t } = useI18n();

const readme = ref<ReadmeResponse | null>(null);
const loading = ref(false);
/** The `code` slug from the error body — `readme.none-stored` and friends. */
const absence = ref<string | null>(null);
const failure = ref<string | null>(null);

async function load() {
  loading.value = true;
  readme.value = null;
  absence.value = null;
  failure.value = null;
  try {
    const { data, error } = await explorePackageReadme({
      path: { registry: props.registry, name: props.name },
      query: props.version ? { version: props.version } : {},
    });
    if (error) {
      // A `404`/`403` here is information, not a failure: the panel renders a
      // statement for each shape rather than an error banner (§4.4).
      const body = error as { code?: string; message?: string };
      absence.value = body.code ?? "readme.none-stored";
      failure.value = body.message ?? null;
      return;
    }
    readme.value = data as ReadmeResponse;
  } catch {
    absence.value = "readme.unreachable";
  } finally {
    loading.value = false;
  }
}

watch(() => [props.registry, props.name, props.version], load, {
  immediate: true,
});

/**
 * The sentence the panel leads with when the text on screen is not the version
 * the reader selected.
 *
 * Spelled out rather than a badge: prose that belongs to different code is the
 * one thing a README panel must never present silently (§4.4).
 */
const fallbackNotice = computed(() => {
  if (!readme.value?.is_fallback || !readme.value.requested_version) return null;
  return t("readmePanel.fallbackFrom", {
    shown: readme.value.version,
    requested: readme.value.requested_version,
  });
});

/**
 * npm's document-root README describes the package, not this version. Saying so
 * is the difference between a fact and a guess presented as one.
 */
const packageLevelNotice = computed(() =>
  readme.value?.package_level ? t("readmePanel.packageLevel") : null,
);

const truncationNotice = computed(() =>
  readme.value?.truncated ? t("readmePanel.truncated") : null,
);

/**
 * A derived answer is read from the cached upstream document rather than from a
 * durable record — which is what makes it possible at all for a version this
 * instance holds no bytes for.
 */
const derivedNotice = computed(() =>
  readme.value && !readme.value.stored ? t("readmePanel.notHeldHere") : null,
);

const absenceMessage = computed(() => {
  switch (absence.value) {
    case "readme.unsupported-type":
      return t("readmePanel.noneForThisRegistryType");
    case "readme.blocked":
      return t("readmePanel.blocked");
    case "readme.unreachable":
      return t("readmePanel.unreachable");
    case "readme.none-stored":
      return t("readmePanel.noneStored");
    default:
      return null;
  }
});
</script>

<template>
  <Card>
    <CardHeader class="pb-2">
      <CardTitle class="text-base flex items-center gap-2">
        <FileText class="h-4 w-4 text-primary shrink-0" />
        {{ t("readmePanel.title") }}
        <span v-if="readme" class="font-mono text-xs text-muted-foreground">
          {{ readme.version }}
        </span>
      </CardTitle>
    </CardHeader>
    <CardContent>
      <p v-if="loading" class="text-sm text-muted-foreground">
        {{ t("common.loading") }}
      </p>

      <template v-else-if="readme">
        <p v-if="fallbackNotice" class="text-sm text-muted-foreground mb-3">
          {{ fallbackNotice }}
        </p>
        <p v-if="packageLevelNotice" class="text-sm text-muted-foreground mb-3">
          {{ packageLevelNotice }}
        </p>
        <p v-if="derivedNotice" class="text-sm text-muted-foreground mb-3">
          {{ derivedNotice }}
        </p>
        <!--
          The one deliberate `v-html` in the console. The rule is right in
          general and wrong here: the HTML is produced and sanitised
          server-side, by an allow-list over an `html5ever` parse with a fuzz
          target and `cargo deny` over it (RFC 0007 §2.6). Re-sanitising in the
          bundle would put the boundary where the fuzz suite cannot reach it.
          `ReadmePanel.test.ts` asserts no third component grows one.
        -->
        <!-- eslint-disable-next-line vue/no-v-html -->
        <div class="readme-body" v-html="readme.rendered_html ?? ''"></div>
        <p v-if="truncationNotice" class="text-xs text-muted-foreground mt-3">
          {{ truncationNotice }}
        </p>
      </template>

      <template v-else>
        <p class="text-sm text-muted-foreground">
          {{ absenceMessage ?? failure ?? t("readmePanel.noneStored") }}
        </p>
      </template>
    </CardContent>
  </Card>
</template>

<style scoped>
/* Typography for the sanitised document, scoped so a README cannot restyle the
   console around it. The sanitiser drops `style` entirely — both the element
   and the attribute — so nothing here can be overridden from the text.
 *
 * Every value is a design token. A README is somebody else's document rendered
 * inside ours, which makes it exactly the place where inventing a type ramp
 * would show: `--t-head`/`--t-sub`/`--t-body`/`--t-meta` are the steps this
 * console has, and `--radius` is zero because "the world has no rounded
 * corner" (tokens.css). */
.readme-body :deep(h1),
.readme-body :deep(h2),
.readme-body :deep(h3) {
  font-weight: 600;
  margin: var(--s3) 0 var(--s2);
  line-height: 1.3;
}
.readme-body :deep(h1) {
  font-size: var(--t-head);
}
.readme-body :deep(h2) {
  font-size: var(--t-sub);
}
.readme-body :deep(h3) {
  font-size: var(--t-row);
}
.readme-body :deep(p),
.readme-body :deep(ul),
.readme-body :deep(ol) {
  margin: var(--s2) 0;
  font-size: var(--t-body);
  line-height: 1.6;
}
.readme-body :deep(ul),
.readme-body :deep(ol) {
  padding-left: var(--s4);
  /* Tailwind's preflight removes list markers; a README's lists are prose and
     need them back. */
  list-style: revert;
}
.readme-body :deep(code) {
  font-family: var(--face-text);
  font-size: var(--t-meta);
  background: var(--ground-raised);
  padding: 0 var(--s1);
}
.readme-body :deep(pre) {
  background: var(--ground-raised);
  padding: var(--s3);
  overflow-x: auto;
  font-size: var(--t-meta);
}
.readme-body :deep(pre code) {
  background: none;
  padding: 0;
}
.readme-body :deep(table) {
  border-collapse: collapse;
  font-size: var(--t-meta);
  /* A wide table scrolls inside its own box rather than widening the page. */
  display: block;
  overflow-x: auto;
}
.readme-body :deep(th),
.readme-body :deep(td) {
  border: 1px solid var(--rule-soft);
  padding: var(--s1) var(--s2);
}
.readme-body :deep(a) {
  text-decoration: underline;
  text-underline-offset: 2px;
}
.readme-body :deep(blockquote) {
  border-left: 2px solid var(--rule-soft);
  padding-left: var(--s3);
  color: var(--ink-dim);
}
/* The chip that replaces a stripped image: the reader sees that an image was
   there and where it pointed, which is the whole of what a badge row says. */
.readme-body :deep(.readme-stripped-image) {
  display: inline-block;
  border: 1px dashed var(--rule-soft);
  padding: 0 var(--s1);
  font-size: var(--t-meta);
  color: var(--ink-dim);
}

/* Under `remote_images = "proxy"` the panel receives real `<img>` tags, pointing
   at this server. It does not know or care which policy produced them — that is
   the point of doing the rewriting server-side (RFC 0007-bis §6.4).

   The styling is written for the failure case, because the endpoint answers 404
   for every image it could not get: a dead URL, a type that is not an image, one
   over the cap, an SVG the sanitiser refused. A broken-image icon would be a
   worse answer than the one `strip` gives, so a broken image is styled to read
   like the chip — dashed, dim, and showing its alt text. */
.readme-body :deep(img) {
  max-width: 100%;
  height: auto;
  vertical-align: middle;
  font-size: var(--t-meta);
  color: var(--ink-dim);
}
.readme-body :deep(img:not([src])) {
  display: none;
}
</style>
