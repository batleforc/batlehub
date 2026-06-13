import { computed, type Ref } from "vue";
import { listRegistries } from "@/client/sdk.gen";
import type { RegistryInfo } from "@/client/types.gen";
import { useApi } from "@/composables/useApi";

/**
 * Resolve the public upstream URL for a package, based on the configured
 * registry's type (e.g. a crates.io link for a `cargo` registry).
 */
export function useUpstreamUrl(registry: Ref<string>, name: Ref<string>, token: Ref<string>) {
  const { data: registriesList } = useApi<RegistryInfo[]>(
    () => listRegistries() as Promise<{ data?: unknown; error?: unknown }>,
    [token],
  );

  const registryType = computed(
    () => registriesList.value?.find((r) => r.name === registry.value)?.type ?? null,
  );

  return computed(() => {
    if (!registry.value || !name.value) return null;
    switch (registryType.value) {
      case "github":
        return `https://github.com/${name.value}`;
      case "npm":
        return `https://www.npmjs.com/package/${name.value}`;
      case "cargo":
        return `https://crates.io/crates/${name.value}`;
      default:
        return null;
    }
  });
}
