<script setup lang="ts">
import { useI18n } from "vue-i18n";
/**
 * The destructive-action contract (RFC 0003 §4.5), enforced in a component
 * rather than left to each caller's judgement.
 *
 * Bulk yank, delete, IP blocks and config reload act on infrastructure other
 * people's builds depend on, so this component refuses to render a bare "Are
 * you sure?":
 *
 *   - **Scope and count before the verb.** `count` and `scope` are required, so
 *     the sentence is always "Yank 47 versions of internal/auth", computed from
 *     the real selection rather than written by hand.
 *   - **Reversibility is stated, not implied.** Yank is reversible; delete is
 *     not; the copy says which, every time.
 *   - **Friction is proportional to consequence.** An irreversible action makes
 *     you type the object's name. A reversible one does not — uniform friction
 *     teaches people to type through the prompt without reading it.
 *
 * The fixed copy is translated; `action`, `itemNoun` and `scope` are still
 * supplied by the caller in English, because they name domain objects and verbs
 * that RFC 0003 §4.6 keeps verbatim. Pluralisation is the naive English rule and
 * belongs to the caller's phrasing, not to this component.
 */
import { computed, ref, watch } from "vue";
import { Dialog } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

const { t } = useI18n();

const props = withDefaults(
  defineProps<{
    open: boolean;
    /** The verb, capitalised: "Delete", "Yank", "Block". */
    action: string;
    /** How many objects the action will touch. Computed from the selection. */
    count: number;
    /** Singular noun for one object: "version", "package", "address". */
    itemNoun: string;
    /** What they belong to: "internal/auth across 2 registries". */
    scope?: string;
    /** Reversible actions skip the typed confirmation. Defaults to false. */
    reversible?: boolean;
    /**
     * The exact string the operator must type when the action is irreversible.
     * Required in that case — a component that let it be omitted would silently
     * become a plain confirm dialog for the most dangerous actions.
     */
    confirmName?: string;
    loading?: boolean;
    error?: string | null;
    /**
     * Let the action be confirmed when `count` is 0.
     *
     * `count > 0` normally gates the button, because for a selection-driven
     * action an empty selection means confirming would do nothing. That reading
     * is wrong when the count *describes* the change rather than *being* the
     * selection: the config-reload Apply acts on the pending reload itself, and
     * `ReloadDiff` only carries registry, access and limits fields — so a change
     * staged for `[server]`, `[storage]` or `[cache]` counts 0 while being
     * entirely real, and the operator was locked out of applying it with only
     * Discard left.
     */
    allowEmpty?: boolean;
  }>(),
  {
    scope: "",
    reversible: false,
    confirmName: "",
    loading: false,
    error: null,
    allowEmpty: false,
  },
);

const emit = defineEmits<{ "update:open": [boolean]; confirm: [] }>();

const typed = ref("");

/** Reset between openings, or a previous confirmation would carry over. */
watch(
  () => props.open,
  (open) => {
    if (!open) typed.value = "";
  },
);

const plural = computed(() => (props.count === 1 ? props.itemNoun : `${props.itemNoun}s`));

const summary = computed(() => {
  const head = `${props.action} ${new Intl.NumberFormat().format(props.count)} ${plural.value}`;
  return props.scope ? `${head} of ${props.scope}` : head;
});

const consequence = computed(() =>
  props.reversible ? t("destructive.canUndo") : t("destructive.cannotUndo"),
);

const needsTypedName = computed(() => !props.reversible && props.confirmName.length > 0);
const nameMatches = computed(() => typed.value.trim() === props.confirmName);
const canConfirm = computed(
  () =>
    !props.loading &&
    (props.allowEmpty || props.count > 0) &&
    (!needsTypedName.value || nameMatches.value),
);

function cancel(): void {
  emit("update:open", false);
}
</script>

<template>
  <Dialog :open="open" @update:open="emit('update:open', $event)">
    <template #title>{{ summary }}</template>
    <template #description>{{ consequence }}</template>

    <div class="space-y-4">
      <!--
        Some destructive actions need an input of their own — a block needs the
        reason a consumer will read back on a 403. Collected here rather than by
        a native `prompt()`, which cannot state count, scope or consequence and
        is not translated.
      -->
      <slot />

      <div v-if="needsTypedName" class="space-y-2">
        <Label :for="'destructive-confirm-name'" class="text-xs text-muted-foreground">
          <i18n-t keypath="destructive.typeToConfirm" tag="span">
            <template #name>
              <span class="font-mono text-foreground">{{ confirmName }}</span>
            </template>
          </i18n-t>
        </Label>
        <Input
          id="destructive-confirm-name"
          v-model="typed"
          autocomplete="off"
          spellcheck="false"
          :aria-invalid="typed.length > 0 && !nameMatches"
          :disabled="loading"
        />
      </div>

      <p v-if="error" class="text-sm text-destructive" role="alert">{{ error }}</p>

      <div class="flex justify-end gap-2">
        <Button variant="outline" size="sm" :disabled="loading" @click="cancel">{{
          t("common.cancel")
        }}</Button>
        <Button variant="destructive" size="sm" :disabled="!canConfirm" @click="emit('confirm')">
          {{ loading ? `${action}…` : action }}
        </Button>
      </div>
    </div>
  </Dialog>
</template>
