import { vi } from "vitest";

// Node's experimental global `localStorage`/`sessionStorage` (undefined unless
// --localstorage-file is passed) and jsdom's own storage implementation are
// both unreliable in this environment, so provide a minimal in-memory Storage
// polyfill and install it on `globalThis` (same object as `window` under
// jsdom), before any test reads or writes it (e.g. via `useAuth`'s `initAuth`).
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

/**
 * Install the real i18n catalogue for every mount.
 *
 * Any component that calls `useI18n()` throws without the plugin, so this would
 * otherwise have to be repeated in every test that mounts anything translated.
 * Using the *real* catalogues rather than a stub is deliberate: assertions then
 * read the strings a user actually sees, and a test that quietly passes against
 * `t('some.key')` echoed back tells us nothing.
 */
import { config } from "@vue/test-utils";
import { i18n } from "@/i18n";

config.global.plugins = [...(config.global.plugins ?? []), i18n];

/*
 * Deliberately *not* `enableAutoUnmount(afterEach)` here.
 *
 * Polling pages leak intervals when a suite never unmounts, so a global hook
 * looks like the right altitude — but vitest runs a file's own `afterEach`
 * before one registered in a setup file, and the suites that mount teleported
 * dialogs already clear `document.body` in theirs. Unmounting after the body is
 * gone makes Vue walk a detached teleport fragment and throw on `nextSibling`;
 * it took 97 tests down. Suites that mount polling pages unmount in their own
 * teardown instead, before clearing the body.
 */
