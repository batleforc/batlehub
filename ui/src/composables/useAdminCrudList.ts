import { ref, computed, watch, type Ref } from "vue";
import { registryHealth } from "@/client/sdk.gen";
import type { RegistryHealthDto } from "@/client/types.gen";
import { useApi, extractMessage } from "@/composables/useApi";
import { useAuth } from "@/composables/useAuth";

interface ApiResult {
  data?: unknown;
  error?: unknown;
}

export interface UseAdminCrudListOptions<TItem, TAddForm> {
  /** Fetch the list of items for the selected registry. */
  listFn: (registry: string) => Promise<ApiResult>;
  /** Submit the "add" form for the selected registry. */
  addFn: (registry: string, form: TAddForm) => Promise<ApiResult>;
  /** Remove the targeted item from the selected registry. */
  removeFn: (registry: string, item: TItem) => Promise<ApiResult>;
  /** Build a fresh "add" form, used initially and after a successful submit. */
  initialAddForm: () => TAddForm;
  /** Optional guard mirroring the "add" button's `:disabled` condition. */
  canSubmitAdd?: (form: TAddForm) => boolean;
}

function apiErrorMessage(apiErr: unknown): string {
  return (apiErr as { message?: string } | null)?.message ?? "API error";
}

/**
 * Shared state/behavior for admin pages that show a registry-scoped list with
 * "add" and "remove" dialogs (e.g. team namespaces, beta channel members).
 */
export function useAdminCrudList<TItem, TAddForm extends object>(
  options: UseAdminCrudListOptions<TItem, TAddForm>,
) {
  const { token } = useAuth();

  const { data: registriesData } = useApi<RegistryHealthDto[]>(
    () => registryHealth() as Promise<ApiResult>,
    [token],
  );

  const registryOptions = computed(() =>
    (registriesData.value ?? []).map((r) => ({ value: r.registry, label: r.registry })),
  );

  const selectedRegistry = ref<string>("");

  watch(registriesData, (list) => {
    if (list && list.length > 0 && !selectedRegistry.value) {
      selectedRegistry.value = list[0].registry;
    }
  });

  const {
    data: items,
    error: itemsError,
    loading: itemsLoading,
    reload: reloadItems,
  } = useApi<TItem[]>(() => {
    if (!selectedRegistry.value) return Promise.resolve({ data: [] });
    return options.listFn(selectedRegistry.value);
  }, [token, selectedRegistry]);

  const addDialogOpen = ref(false);
  const addForm = ref(options.initialAddForm()) as Ref<TAddForm>;
  const addLoading = ref(false);
  const addError = ref<string | null>(null);

  async function submitAdd() {
    if (!selectedRegistry.value) return;
    if (options.canSubmitAdd && !options.canSubmitAdd(addForm.value)) return;
    addLoading.value = true;
    addError.value = null;
    try {
      const { error: apiErr } = await options.addFn(selectedRegistry.value, addForm.value);
      if (apiErr) throw new Error(apiErrorMessage(apiErr));
      addDialogOpen.value = false;
      addForm.value = options.initialAddForm();
      reloadItems();
    } catch (e) {
      addError.value = extractMessage(e);
    } finally {
      addLoading.value = false;
    }
  }

  const removeTarget = ref(null) as Ref<TItem | null>;
  const removeLoading = ref(false);
  const removeError = ref<string | null>(null);

  async function confirmRemove() {
    if (!removeTarget.value || !selectedRegistry.value) return;
    removeLoading.value = true;
    removeError.value = null;
    try {
      const { error: apiErr } = await options.removeFn(selectedRegistry.value, removeTarget.value);
      if (apiErr) throw new Error(apiErrorMessage(apiErr));
      removeTarget.value = null;
      reloadItems();
    } catch (e) {
      removeError.value = extractMessage(e);
    } finally {
      removeLoading.value = false;
    }
  }

  return {
    registryOptions,
    selectedRegistry,
    items,
    itemsError,
    itemsLoading,
    reloadItems,
    addDialogOpen,
    addForm,
    addLoading,
    addError,
    submitAdd,
    removeTarget,
    removeLoading,
    removeError,
    confirmRemove,
  };
}
