const TTL_MS = 5 * 60 * 1_000; // 5 minutes

interface CacheEntry<T> {
  data: T;
  expiresAt: number;
}

const _store = new Map<string, CacheEntry<unknown>>();

function key(registry: string, page: number, sort: string, query: string): string {
  return `${registry}::${page}::${sort}::${query}`;
}

function invalidate(registry?: string): void {
  if (!registry) {
    _store.clear();
    return;
  }
  for (const k of _store.keys()) {
    if (k.startsWith(`${registry}::`)) _store.delete(k);
  }
}

export function useExploreCache<T>() {
  function get(registry: string, page: number, sort: string, query: string): T | undefined {
    const entry = _store.get(key(registry, page, sort, query)) as CacheEntry<T> | undefined;
    if (!entry) return undefined;
    if (Date.now() > entry.expiresAt) {
      _store.delete(key(registry, page, sort, query));
      return undefined;
    }
    return entry.data;
  }

  function set(registry: string, page: number, sort: string, query: string, data: T): void {
    _store.set(key(registry, page, sort, query), {
      data,
      expiresAt: Date.now() + TTL_MS,
    });
  }

  return { get, set, invalidate };
}
