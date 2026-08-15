import { ref, watch, type Ref } from "vue";

/**
 * A ref these composables only ever *read*.
 *
 * Widening the parameter types to this is what lets a caller pass a getter ref
 * or a `computed` — which is the only form that survives a form object being
 * replaced wholesale, as `openCreate`/`openEdit` dialogs do. A plain
 * `Ref<T>` remains assignable, so no existing caller changes.
 */
type ReadonlyRef<T> = Readonly<Ref<T>>;

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
  query: ReadonlyRef<string>,
  fetcher: (q: string) => Promise<T[]>,
  opts: {
    enabled?: ReadonlyRef<boolean>;
    minLength?: number;
    /**
     * Extra reactive inputs the *fetcher* reads but the query does not name —
     * a registry filter, say. They belong in the same watcher rather than a
     * separate one, so that changing one re-issues the request through `run`
     * and bumps `seq`. A sibling `watch` that only cleared `items` would leave
     * an in-flight request free to land afterwards with results computed under
     * the old value, and would never refetch under the new one.
     */
    deps?: ReadonlyRef<unknown>[];
  } = {},
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

  const depCount = opts.deps?.length ?? 0;

  watch(
    [query, opts.enabled ?? ref(true), ...(opts.deps ?? [])],
    (next, prev) => {
      if (timer) clearTimeout(timer);
      const text = String(next[0] ?? "");
      const enabled = Boolean(next[1]);

      if (!enabled || text.trim().length < minLength) {
        // Cancel in flight too: a request issued for `lo` must not repopulate
        // the list after the operator has cleared the field.
        seq++;
        items.value = [];
        loading.value = false;
        return;
      }

      // A *dep* change (not a keystroke) invalidates what is on screen now
      // rather than merely making it stale: the visible list was computed under
      // the old registry and names packages that may not exist under the new
      // one. Bumping `seq` strands any in-flight response as well, so it cannot
      // land afterwards and repopulate the list from the old filter.
      //
      // Deliberately not done on a keystroke: clearing there would blank the
      // list for the whole debounce and flash "nothing matches" while the
      // operator is still typing.
      const depsChanged =
        depCount > 0 &&
        prev !== undefined &&
        next.slice(2, 2 + depCount).some((v, i) => v !== prev[2 + i]);
      if (depsChanged) {
        seq++;
        items.value = [];
      }

      timer = setTimeout(() => void run(text.trim()), DEBOUNCE_MS);
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
export function usePackageNameSuggestions(
  query: ReadonlyRef<string>,
  registry: ReadonlyRef<string>,
) {
  // `registry` goes in as a dep rather than a sibling `watch` that only cleared
  // the list. The fetcher reads it, so it has to participate in the same
  // last-response-wins sequence: clearing alone left an in-flight request free
  // to land under the new registry with hits from the old one, and never
  // refetched, so the field reported "nothing matches" for a registry that does
  // have matches until the operator typed another character.
  const { items, loading } = suggest<ComboboxOption>(
    query,
    async (q) => {
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
    },
    { deps: [registry] },
  );

  return { options: items, loading };
}

/**
 * The versions of one package — a closed set, known once its parent fields are
 * answered, and small enough to load whole.
 */
export function useVersionSuggestions(registry: ReadonlyRef<string>, name: ReadonlyRef<string>) {
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
export function useSubjectSuggestions(query: ReadonlyRef<string>) {
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
