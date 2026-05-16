import { ref, watchEffect, type Ref } from "vue";

function extractMessage(err: unknown): string {
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
  let tick = ref(0);

  async function run() {
    loading.value = true;
    error.value = null;
    try {
      const result = await fn();
      if (result.error) {
        error.value = extractMessage(result.error);
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

  watchEffect(() => {
    // Track deps and tick so watchers fire on reload() too.
    deps.forEach((d) => d.value);
    tick.value;
    run();
  });

  function reload() {
    tick.value++;
  }

  return { data, error, loading, reload };
}
