import { vi } from "vitest";

// Node's experimental global `localStorage`/`sessionStorage` (undefined unless
// --localstorage-file is passed) and jsdom's own storage implementation are
// both unreliable in this environment, so provide a minimal in-memory Storage
// polyfill and install it on `globalThis` (same object as `window` under
// jsdom). `useAuth.ts` reads `localStorage` at module scope, so this must run
// before any test module imports it.
class MemoryStorage implements Storage {
  readonly #store = new Map<string, string>();

  get length(): number {
    return this.#store.size;
  }

  clear(): void {
    this.#store.clear();
  }

  getItem(key: string): string | null {
    return this.#store.has(key) ? this.#store.get(key)! : null;
  }

  key(index: number): string | null {
    return Array.from(this.#store.keys())[index] ?? null;
  }

  removeItem(key: string): void {
    this.#store.delete(key);
  }

  setItem(key: string, value: string): void {
    this.#store.set(key, String(value));
  }
}

for (const key of ["localStorage", "sessionStorage"] as const) {
  Object.defineProperty(globalThis, key, {
    value: new MemoryStorage(),
    writable: true,
    configurable: true,
  });
}

globalThis.matchMedia ??= (query: string) => ({
  matches: false,
  media: query,
  onchange: null,
  addListener: () => {},
  removeListener: () => {},
  addEventListener: () => {},
  removeEventListener: () => {},
  dispatchEvent: () => false,
});

// jsdom has no layout engine, so these callbacks would never fire anyway.
class ResizeObserverStub {
  observe() {
    // no-op
  }
  unobserve() {
    // no-op
  }
  disconnect() {
    // no-op
  }
}
globalThis.ResizeObserver ??= ResizeObserverStub as unknown as typeof ResizeObserver;

Element.prototype.scrollIntoView ??= vi.fn();
Element.prototype.hasPointerCapture ??= () => false;
Element.prototype.releasePointerCapture ??= () => {};
Element.prototype.setPointerCapture ??= () => {};
