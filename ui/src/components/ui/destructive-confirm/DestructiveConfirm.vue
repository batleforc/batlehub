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
 * The fixed copy is translated, and so is the sentence that joins it: `of` was
 * concatenated here, which put one English word in the middle of six otherwise
 * French confirmations.
 *
 * `action`, `itemNoun` and `scope` are the caller's words, and every caller
 * now passes them through the catalogue. Pluralisation went with them: this
 * component appended an `s`, which read as the naive English rule and was
 * applied to `paquet`, `modification` and `artefact en cache`. It happens to
 * produce the right letters for those three and the wrong ones for the next
 * noun somebody adds, and no rule about morphology belongs in a dialog anyway
 * — so a caller whose count can exceed one supplies `itemNounPlural`, from the
 * same catalogue as the singular.
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
    /**
     * Plural of `itemNoun`, for the callers whose `count` can exceed one.
     * Defaults to the singular rather than to a made-up plural: a repeated
     * singular reads as a missing translation, an invented one reads as a
     * word.
     */
    itemNounPlural?: string;
    /** What they belong to: "internal/auth across 2 registries". */
    scope?: string;
    /** Reversible actions skip the typed confirmation. Defaults to false. */
    reversible?: boolean;
    /**
     * What this particular action costs, when the two stock sentences do not
     * say it.
     *
     * `destructive.cannotUndo` reads "The artifacts and their metadata are
     * removed permanently", which is true of a delete and false of everything
     * else that is irreversible — a revoked token removes no artifact, and a
     * forced config reload and an audit-log purge remove none either. A caller
     * whose consequence is different says it here rather than shipping a
     * sentence about artifacts over an action that touches none.
     *
     * Optional for now. It should become *required* whenever `reversible` is
     * false, which is a change to every irreversible call site and belongs in
     * its own commit.
     */
    consequence?: string;
    /**
     * The exact string the operator must type when the action is irreversible.
     * Required in that case — a component that let it be omitted would silently
     * become a plain confirm dialog for the most dangerous actions.
     */
    confirmName?: string;
    /**
     * Compare `confirmName` without regard to case.
     *
     * Off by default, because most call sites type back an *identifier* — a
     * package name, a token name — where `Foo` and `foo` are two different
     * objects and accepting either would confirm the action against something
     * the operator did not name.
     *
     * On for the call sites whose confirmation is a *keyword* ("reload",
     * "purge", "delete"): there is nothing for the case to disambiguate, so a
     * capitalised word — which is what a phone keyboard and a habit of typing
     * commands both produce — was rejected with no explanation beyond the
     * button staying grey.
     */
    confirmCaseInsensitive?: boolean;
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
    itemNounPlural: "",
    scope: "",
    reversible: false,
    consequence: "",
    confirmName: "",
    confirmCaseInsensitive: false,
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

const plural = computed(() =>
  props.count === 1 ? props.itemNoun : props.itemNounPlural || props.itemNoun,
);

const summary = computed(() => {
  const parts = {
    action: props.action,
    count: new Intl.NumberFormat().format(props.count),
    item: plural.value,
    scope: props.scope,
  };
  return props.scope ? t("destructive.summaryOf", parts) : t("destructive.summary", parts);
});

const consequenceText = computed(
  () => props.consequence || t(props.reversible ? "destructive.canUndo" : "destructive.cannotUndo"),
);

const needsTypedName = computed(() => !props.reversible && props.confirmName.length > 0);
/**
 * Case-folding is done by a collator rather than `toLowerCase()`: the words are
 * translated, and the naive fold is the one that turns Turkish `İ` into two code
 * points that no longer equal the `i` the operator typed. `sensitivity: "accent"`
 * ignores case and keeps accents, so `purger` still differs from `purgér`.
 */
const caseFoldingCollator = new Intl.Collator(undefined, { sensitivity: "accent" });

const nameMatches = computed(() => {
  const entered = typed.value.trim();
  return props.confirmCaseInsensitive
    ? caseFoldingCollator.compare(entered, props.confirmName) === 0
    : entered === props.confirmName;
});
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
    <template #description>{{ consequenceText }}</template>

    <div class="space-y-4">
      <!--
        Some destructive actions need an input of their own — a block needs the
        reason a consumer will read back on a 403. Collected here rather than by
        a native `prompt()`, which cannot state count, scope or consequence and
        is not translated.
      -->
      <slot />

      <div v-if="needsTypedName" class="space-y-2">
        <Label for="destructive-confirm-name" class="text-xs text-muted-foreground">
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
