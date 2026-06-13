import { createApp, type App } from "vue";

/** Run a composable inside a minimal component so lifecycle hooks (onMounted, watchEffect, ...) work. */
export function withSetup<T>(composable: () => T): [T, App] {
  let result!: T;
  const app = createApp({
    setup() {
      result = composable();
      return () => null;
    },
  });
  app.mount(document.createElement("div"));
  return [result, app];
}
