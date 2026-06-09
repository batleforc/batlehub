<script setup lang="ts">
import { ref, onMounted, computed } from "vue";
import { useRouter, useRoute } from "vue-router";
import { client } from "@/client/client.gen";
import { me, listOidcProviders } from "@/client/sdk.gen";
import type { MeResponse, OidcProviderInfo } from "@/client/types.gen";
import { useAuth, storeTokens } from "@/composables/useAuth";
import { generateOidcState } from "@/router";
import { API_BASE_URL } from "@/config";
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";

const router = useRouter();
const route = useRoute();
const { isAuthenticated } = useAuth();

const inputToken = ref("");
const error = ref<string | null>(
  // Surface errors forwarded from the OIDC callback.
  typeof route.query.error === "string" ? route.query.error : null,
);
const loading = ref(false);
const oidcLoadingProvider = ref<string | null>(null);
const oidcProviders = ref<OidcProviderInfo[]>([]);

const redirect = computed(() =>
  typeof route.query.redirect === "string" ? route.query.redirect : "/packages",
);

// Already signed in → skip the login page.
onMounted(async () => {
  if (isAuthenticated.value) {
    router.replace(redirect.value);
    return;
  }
  // Fetch the list of available OIDC providers.
  try {
    const result = await listOidcProviders();
    oidcProviders.value = (result.data as OidcProviderInfo[] | undefined) ?? [];
  } catch {
    oidcProviders.value = [];
  }
});

async function submit() {
  const tok = inputToken.value.trim();
  if (!tok) {
    error.value = "Please enter a token.";
    return;
  }

  loading.value = true;
  error.value = null;

  // Probe the token before persisting it.
  client.setConfig({ auth: tok });
  try {
    const result = await me();
    const id = result.data as MeResponse | undefined;
    if (id && id.role !== "anonymous") {
      storeTokens(tok); // no refresh token or expiry for static tokens
      router.push(redirect.value);
    } else {
      error.value = "Token is valid but grants only anonymous access.";
      client.setConfig({ auth: undefined });
    }
  } catch {
    error.value = "Invalid token — check the value and try again.";
    client.setConfig({ auth: undefined });
  } finally {
    loading.value = false;
  }
}

function signInWithOidc(providerName: string) {
  oidcLoadingProvider.value = providerName;
  // Generate a random state, store in sessionStorage for CSRF validation,
  // then hand off to the backend which threads it through to the provider.
  const state = generateOidcState();
  const params = new URLSearchParams({ state, provider: providerName });
  globalThis.location.href = `${API_BASE_URL}/api/v1/auth/oidc/login?${params}`;
}

function continueAnonymously() {
  const dest = redirect.value === "/login" ? "/packages" : redirect.value;
  router.push(dest);
}

function providerLabel(name: string): string {
  // Capitalise and replace dashes/underscores with spaces for display.
  return name.replace(/[-_]/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
}
</script>

<template>
  <div class="flex min-h-[calc(100vh-3.5rem)] items-center justify-center px-4 cyber-grid-bg">
    <Card class="w-full max-w-sm [box-shadow:var(--cyber-glow)]">
      <CardHeader class="space-y-1">
        <CardTitle class="font-mono text-2xl font-bold cyber-text-glow"> Sign in </CardTitle>
        <CardDescription> Authenticate to access protected resources. </CardDescription>
      </CardHeader>

      <CardContent class="space-y-4">
        <!-- OIDC sign-in buttons (one per configured provider) -->
        <template v-if="oidcProviders.length > 0">
          <Button
            v-for="p in oidcProviders"
            :key="p.name"
            type="button"
            variant="outline"
            class="w-full"
            :disabled="oidcLoadingProvider !== null"
            @click="signInWithOidc(p.name)"
          >
            {{
              oidcLoadingProvider === p.name
                ? "Redirecting…"
                : oidcProviders.length === 1
                  ? "Sign in with OIDC"
                  : `Sign in with ${providerLabel(p.name)}`
            }}
          </Button>

          <!-- Divider between OIDC and token form -->
          <div class="relative">
            <div class="absolute inset-0 flex items-center">
              <span class="w-full border-t border-border/60" />
            </div>
            <div class="relative flex justify-center text-xs uppercase">
              <span class="bg-card px-2 font-mono text-muted-foreground tracking-widest"
                >or use a token</span
              >
            </div>
          </div>
        </template>

        <!-- Static-token form -->
        <form class="space-y-4" @submit.prevent="submit">
          <div class="space-y-2">
            <Label for="token">Bearer token</Label>
            <Input
              id="token"
              v-model="inputToken"
              type="password"
              placeholder="Paste your token here"
              autocomplete="current-password"
            />
          </div>

          <p v-if="error" class="text-sm text-destructive">
            {{ error }}
          </p>

          <Button type="submit" class="w-full" :disabled="loading">
            {{ loading ? "Signing in…" : "Sign in with token" }}
          </Button>
        </form>

        <Button type="button" variant="ghost" class="w-full" @click="continueAnonymously">
          Continue without signing in
        </Button>
      </CardContent>
    </Card>
  </div>
</template>
