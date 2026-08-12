import { ref, watchEffect, type Ref } from "vue";

/**
 * Deliberately not a catalogue key: this fires when the API is unreachable or
 * misrouted, which is a deployment fault the operator has to read, and it sits
 * beside the file's existing "Unknown error" literal. Recorded in RFC 0003 §13.
 */
const NOT_JSON =
  "The API returned a non-JSON response. Check VITE_API_BASE_URL and that /api is routed to the backend.";

export function extractMessage(err: unknown): string {
  if (err == null) return "Unknown error";
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  if (typeof err === "object") {
    const e = err as Record<string, unknown>;
    if (typeof e["message"] === "string") return e["message"];
    if (typeof e["error"] === "string") return e["error"];
  }
  return String(err);
}

interface ApiState<T> {
  data: Ref<T | null>;
  error: Ref<string | null>;
  loading: Ref<boolean>;
  reload: () => void;
}

export function useApi<T>(
  fn: () => Promise<{ data?: unknown; error?: unknown }>,
  deps: Ref<unknown>[] = [],
): ApiState<T> {
  const data = ref<T | null>(null) as Ref<T | null>;
  const error = ref<string | null>(null);
  const loading = ref(false);
  const tick = ref(0);

  async function run() {
    loading.value = true;
    error.value = null;
    try {
      const result = await fn();
      if (result.error) {
        error.value = extractMessage(result.error);
        data.value = null;
      } else if (typeof result.data === "string") {
        /*
         * Every endpoint in this API answers with JSON, so a string here means
         * something else replied: a reverse proxy's HTML error page, or the
         * SPA's own index.html when `/api` is not routed to the backend. The
         * generated client hands that text back as `data`, and a component then
         * calls `.filter()` on a string and throws during render — which kills
         * the page rather than showing anything.
         *
         * That is not hypothetical: it is what a production build served
         * without an API does, and it is why the CI rendered gate was scanning
         * an empty shell while reporting the pages clean.
         */
        error.value = NOT_JSON;
        data.value = null;
      } else {
        data.value = result.data as T;
      }
    } catch (e) {
      error.value = extractMessage(e);
      data.value = null;
    } finally {
      loading.value = false;
    }
  }

  const watched = [...deps, tick];

  watchEffect(() => {
    watched.forEach((d) => d.value);
    void run();
  });

  function reload() {
    tick.value++;
  }

  return { data, error, loading, reload };
}
