import { ref, computed, watch } from "vue";
import { client } from "@/client/client.gen";
import { me, oidcRefresh } from "@/client/sdk.gen";
import type { MeResponse, RefreshResponse } from "@/client/types.gen";

const ACCESS_TOKEN_KEY = "proxy_cache_access_token";
const REFRESH_TOKEN_KEY = "proxy_cache_refresh_token";
const EXPIRES_AT_KEY = "proxy_cache_token_expires_at"; // unix ms

// ── Singleton state ────────────────────────────────────────────────────────────

const token = ref<string>(localStorage.getItem(ACCESS_TOKEN_KEY) ?? "");
const refreshToken = ref<string>(localStorage.getItem(REFRESH_TOKEN_KEY) ?? "");
const expiresAt = ref<number>(
  Number(localStorage.getItem(EXPIRES_AT_KEY) ?? "0"),
);
const identity = ref<MeResponse | null>(null);
const identityReady = ref(false);

// ── Token persistence ──────────────────────────────────────────────────────────

watch(token, (val) => {
  if (val) localStorage.setItem(ACCESS_TOKEN_KEY, val);
  else localStorage.removeItem(ACCESS_TOKEN_KEY);
  client.setConfig({ auth: val || undefined });
});

watch(refreshToken, (val) => {
  if (val) localStorage.setItem(REFRESH_TOKEN_KEY, val);
  else localStorage.removeItem(REFRESH_TOKEN_KEY);
});

watch(expiresAt, (val) => {
  if (val) localStorage.setItem(EXPIRES_AT_KEY, String(val));
  else localStorage.removeItem(EXPIRES_AT_KEY);
});

// ── Identity fetch ─────────────────────────────────────────────────────────────

async function refreshIdentity() {
  identityReady.value = false;
  if (!token.value) {
    identity.value = null;
    identityReady.value = true;
    return;
  }
  try {
    const result = await me();
    identity.value = (result.data as MeResponse | undefined) ?? null;
  } catch {
    identity.value = null;
  }
  identityReady.value = true;
}

// Run on startup.
client.setConfig({ auth: token.value || undefined });
refreshIdentity();

// Re-fetch identity whenever the access token changes.
watch(token, () => refreshIdentity());

// ── Token refresh ──────────────────────────────────────────────────────────────

let _refreshTimer: ReturnType<typeof setTimeout> | null = null;

function scheduleRefresh() {
  if (_refreshTimer) clearTimeout(_refreshTimer);
  if (!expiresAt.value || !refreshToken.value) return;

  // Refresh 60 s before the token expires.
  const msUntilRefresh = expiresAt.value - Date.now() - 60_000;
  if (msUntilRefresh <= 0) {
    // Already expired or about to — refresh immediately.
    doRefresh();
    return;
  }
  _refreshTimer = setTimeout(doRefresh, msUntilRefresh);
}

async function doRefresh() {
  if (!refreshToken.value) return;
  try {
    const result = await oidcRefresh({ body: { refresh_token: refreshToken.value } });
    if (!result.data) throw new Error("empty refresh response");
    const data = result.data as RefreshResponse;
    storeTokens(
      data.access_token,
      data.refresh_token ?? refreshToken.value,
      data.expires_in,
    );
  } catch (e) {
    console.warn("[auth] token refresh failed:", e);
    // Don't force logout — let the next API call surface the 401.
  }
}

/**
 * Store a new set of tokens received from OIDC (initial login or refresh).
 * `expiresInSeconds` is the `expires_in` value from the token response.
 */
export function storeTokens(
  accessToken: string,
  newRefreshToken?: string | null,
  expiresInSeconds?: number | null,
) {
  token.value = accessToken;
  if (newRefreshToken) refreshToken.value = newRefreshToken;
  if (expiresInSeconds) {
    expiresAt.value = Date.now() + expiresInSeconds * 1000;
    scheduleRefresh();
  } else {
    expiresAt.value = 0;
  }
}

// Schedule refresh on startup if we already have tokens with a known expiry.
scheduleRefresh();

// ── Logout ─────────────────────────────────────────────────────────────────────

function logout() {
  if (_refreshTimer) clearTimeout(_refreshTimer);
  token.value = "";
  refreshToken.value = "";
  expiresAt.value = 0;
  identity.value = null;
}

// ── Computed ───────────────────────────────────────────────────────────────────

const isAdmin = computed(() => identity.value?.role === "admin");
const isAuthenticated = computed(
  () => !!token.value && identity.value?.role !== "anonymous",
);

export function useAuth() {
  return {
    token,
    refreshToken,
    expiresAt,
    identity,
    identityReady,
    isAdmin,
    isAuthenticated,
    logout,
    doRefresh,
  };
}
