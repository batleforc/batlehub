import { ref, watch, type Ref } from "vue";

import { explorePackageDetail, explorePackages, listSubjects } from "@/client/sdk.gen";
import type {
  ExplorePackageDetailResponse,
  ExplorePackageListResponse,
  SubjectsResponse,
} from "@/client/types.gen";
import type { ComboboxOption } from "@/components/ui/combobox";

/**
 * The suggestion sources behind RFC 0004-bis §6.2's field sweep.
 *
 * Fourteen fields asked an operator to type an identifier the server already
 * knew. `PackageFilter::name_contains` — a paginated substring filter — has
 * shipped since the repository port was written, three endpoints expose it, and
 * no field in the console used it to suggest anything.
 *
 * Everything here is debounced and every source is *local*: `name_contains` and
 * the package detail read this instance's own database, and `/admin/subjects`
 * reads identities it has already seen. Nothing in this file reaches a third
 * party. `explore/upstream` is deliberately absent — it is an explicit
 * affordance the caller renders, not something a keystroke triggers (O4).
 */

/** Long enough that a fast typist issues one request, short enough to feel live. */
const DEBOUNCE_MS = 250;

/** Below this a substring match is every package the instance has. */
const MIN_QUERY = 2;

const MAX_SUGGESTIONS = 10;

/**
 * Drive `fetcher` from `query`, debounced, with the last-response-wins guard
 * every one of these needs.
 *
 * The guard is not optional here for the same reason it was not optional on
 * `/packages` (§6.3): these fetches are issued per keystroke, so an earlier,
 * slower response landing last is the *common* case rather than a race that
 * needs luck to hit.
 */
function suggest<T>(
  query: Ref<string>,
  fetcher: (q: string) => Promise<T[]>,
  opts: { enabled?: Ref<boolean>; minLength?: number } = {},
) {
  const items = ref<T[]>([]) as Ref<T[]>;
  const loading = ref(false);
  const minLength = opts.minLength ?? MIN_QUERY;

  let timer: ReturnType<typeof setTimeout> | null = null;
  let seq = 0;

  async function run(q: string) {
    const mine = ++seq;
    loading.value = true;
    try {
      const result = await fetcher(q);
      if (mine !== seq) return; // superseded by a later keystroke
      items.value = result;
    } catch {
      // A suggestion source that fails must not break the field it decorates:
      // the operator can still type. Silence is correct *here* precisely
      // because the field never depended on the answer.
      if (mine === seq) items.value = [];
    } finally {
      if (mine === seq) loading.value = false;
    }
  }

  watch(
    [query, opts.enabled ?? ref(true)] as const,
    ([q, enabled]) => {
      if (timer) clearTimeout(timer);
      if (!enabled || q.trim().length < minLength) {
        // Cancel in flight too: a request issued for `lo` must not repopulate
        // the list after the operator has cleared the field.
        seq++;
        items.value = [];
        loading.value = false;
        return;
      }
      timer = setTimeout(() => void run(q.trim()), DEBOUNCE_MS);
    },
    { immediate: true },
  );

  return { items, loading };
}

/**
 * Package names this instance has, matching a substring.
 *
 * `/api/v1/explore/packages` rather than the admin listing: it is the one every
 * viewer can reach, so the same field works on `/tools/access-check` and on an
 * admin page without two code paths that must agree.
 */
export function usePackageNameSuggestions(query: Ref<string>, registry: Ref<string>) {
  const { items, loading } = suggest<ComboboxOption>(query, async (q) => {
    const { data } = await explorePackages({
      query: {
        name: q,
        registry: registry.value || undefined,
        per_page: MAX_SUGGESTIONS,
        page: 0,
      },
    });
    const body = data as ExplorePackageListResponse | undefined;
    return (body?.items ?? []).map((entry) => ({
      value: entry.name,
      hint: entry.registry,
    }));
  });

  // A registry change makes the current suggestions wrong, not merely stale.
  watch(registry, () => {
    items.value = [];
  });

  return { options: items, loading };
}

/**
 * The versions of one package — a closed set, known once its parent fields are
 * answered, and small enough to load whole.
 */
export function useVersionSuggestions(registry: Ref<string>, name: Ref<string>) {
  const options = ref<ComboboxOption[]>([]);
  const loading = ref(false);
  let seq = 0;

  watch(
    [registry, name] as const,
    async ([reg, pkg]) => {
      const mine = ++seq;
      if (!reg || !pkg.trim()) {
        options.value = [];
        return;
      }
      loading.value = true;
      try {
        /* The *explore* detail, not the admin one: every viewer can reach it,
           so the public `/tools/access-check` and the admin simulator share one
           source instead of two that have to agree. */
        const { data } = await explorePackageDetail({
          path: { registry: reg, name: pkg.trim() },
        });
        if (mine !== seq) return;
        const body = data as ExplorePackageDetailResponse | undefined;
        // Newest first: the version an operator is asking about is far more
        // often the latest than the oldest.
        options.value = (body?.versions ?? []).map((v) => ({ value: v.version })).reverse();
      } catch {
        if (mine === seq) options.value = [];
      } finally {
        if (mine === seq) loading.value = false;
      }
    },
    { immediate: true },
  );

  /** Why the field is dark, so it is not merely dark. */
  const ready = () => !!registry.value && !!name.value.trim();

  return { options, loading, ready };
}

/**
 * Identities this instance has seen (A8).
 *
 * The field these feed is the one whose failure was silent: filtering the audit
 * log for `alice` on an instance that stores `oidc:alice` returned an empty
 * table, which reads exactly like "this user did nothing".
 */
export function useSubjectSuggestions(query: Ref<string>) {
  const { items, loading } = suggest<ComboboxOption>(
    query,
    async (q) => {
      const { data } = await listSubjects({ query: { q, limit: MAX_SUGGESTIONS } });
      const body = data as SubjectsResponse | undefined;
      return (body?.items ?? []).map((s) => ({
        value: s.user_id,
        hint: s.sources.join(", "),
      }));
    },
    // One character is enough: subjects are few, and the prefix an operator
    // types (`a`) is often shorter than the stored form (`oidc:alice`).
    { minLength: 1 },
  );

  return { options: items, loading };
}
