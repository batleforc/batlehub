import { ref, watch } from "vue";
import { client } from "@/client/client.gen";

const TOKEN_KEY = "proxy_cache_token";

const token = ref<string>(localStorage.getItem(TOKEN_KEY) ?? "");

watch(token, (val) => {
  if (val) {
    localStorage.setItem(TOKEN_KEY, val);
    client.setConfig({ auth: val });
  } else {
    localStorage.removeItem(TOKEN_KEY);
    client.setConfig({ auth: undefined });
  }
}, { immediate: true });

export function useAuth() {
  return { token };
}
