import { useAuth } from "@/composables/useAuth";

export function useAuthFetch() {
  const { token } = useAuth();

  function authHeaders(): Record<string, string> {
    return token.value ? { Authorization: `Bearer ${token.value}` } : {};
  }

  async function authFetch(url: string, init: RequestInit = {}): Promise<Response> {
    return fetch(url, {
      ...init,
      headers: { ...authHeaders(), ...(init.headers as Record<string, string> | undefined) },
    });
  }

  return { authHeaders, authFetch };
}
