import { ref, computed, watch } from "vue";
import { client } from "@/client/client.gen";
import { me, oidcRefresh } from "@/client/sdk.gen";
import type { MeResponse } from "@/client/types.gen";

const ACCESS_TOKEN_KEY = "batlehub_access_token";
const REFRESH_TOKEN_KEY = "batlehub_refresh_token";
const EXPIRES_AT_KEY = "batlehub_token_expires_at"; // unix ms
const OIDC_PROVIDER_KEY = "batlehub_oidc_provider";

// ── Singleton state ────────────────────────────────────────────────────────────
//
// Refs start empty; `initAuth()` populates them from `localStorage` and kicks off
// the initial identity fetch. Nothing here runs at import time — see `initAuth`.

const token = ref<string>("");
const refreshToken = ref<string>("");
const expiresAt = ref<number>(0);
const oidcProvider = ref<string>("");
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

watch(oidcProvider, (val) => {
  if (val) localStorage.setItem(OIDC_PROVIDER_KEY, val);
  else localStorage.removeItem(OIDC_PROVIDER_KEY);
});

// ── Identity fetch ─────────────────────────────────────────────────────────────

async function refreshIdentity() {
  identityReady.value = false;
  try {
    const result = await me();
    identity.value = result.data ?? null;
  } catch {
    identity.value = null;
  }
  identityReady.value = true;
}

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
    void doRefresh();
    return;
  }
  _refreshTimer = setTimeout(doRefresh, msUntilRefresh);
}

async function doRefresh() {
  if (!refreshToken.value) return;
  try {
    const result = await oidcRefresh({
      body: {
        refresh_token: refreshToken.value,
        provider: oidcProvider.value || undefined,
      },
    });
    if (!result.data) throw new Error("empty refresh response");
    const data = result.data;
    storeTokens(
      data.access_token,
      data.refresh_token ?? refreshToken.value,
      data.expires_in,
      oidcProvider.value || undefined,
    );
  } catch (e) {
    // Don't force logout — let the next API call surface the 401.
    console.error("Failed to refresh token:", e);
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
  provider?: string | null,
) {
  token.value = accessToken;
  if (newRefreshToken) refreshToken.value = newRefreshToken;
  if (provider != null) oidcProvider.value = provider;
  if (expiresInSeconds) {
    expiresAt.value = Date.now() + expiresInSeconds * 1000;
    scheduleRefresh();
  } else {
    expiresAt.value = 0;
  }
}

// ── Startup ────────────────────────────────────────────────────────────────────

let initialized = false;

/**
 * Load persisted tokens from `localStorage`, point the API client at them, kick
 * off the initial identity fetch, and schedule an automatic refresh if needed.
 *
 * Must be called exactly once, after the API client's `baseUrl` is configured
 * (see `clientInit.ts`) and before any component reads `useAuth()` state. A no-op
 * on repeat calls.
 */
export function initAuth() {
  if (initialized) return;
  initialized = true;

  const storedToken = localStorage.getItem(ACCESS_TOKEN_KEY) ?? "";
  refreshToken.value = localStorage.getItem(REFRESH_TOKEN_KEY) ?? "";
  expiresAt.value = Number(localStorage.getItem(EXPIRES_AT_KEY) ?? "0");
  oidcProvider.value = localStorage.getItem(OIDC_PROVIDER_KEY) ?? "";

  client.setConfig({ auth: storedToken || undefined });
  token.value = storedToken;
  // Assigning `token` above only re-triggers `refreshIdentity` (via the `watch`
  // below) when the stored value differs from the ref's initial "" — i.e. when
  // there was a token to restore. An empty store needs an explicit kick so
  // `identity`/`identityReady` still settle for anonymous visitors.
  if (!storedToken) void refreshIdentity();
  scheduleRefresh();
}

// ── Logout ─────────────────────────────────────────────────────────────────────

function logout() {
  if (_refreshTimer) clearTimeout(_refreshTimer);
  token.value = "";
  refreshToken.value = "";
  expiresAt.value = 0;
  oidcProvider.value = "";
  identity.value = null;
}

// ── Computed ───────────────────────────────────────────────────────────────────

const isAdmin = computed(() => identity.value?.role === "admin");
const isAuthenticated = computed(() => !!token.value && identity.value?.role !== "anonymous");

export function useAuth() {
  return {
    token,
    refreshToken,
    expiresAt,
    oidcProvider,
    identity,
    identityReady,
    isAdmin,
    isAuthenticated,
    logout,
    doRefresh,
  };
}
