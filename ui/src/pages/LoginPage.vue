<script setup lang="ts">
import { ref, onMounted, computed } from "vue";
import { useRouter, useRoute } from "vue-router";
import { client } from "@/client/client.gen";
import { me } from "@/client/sdk.gen";
import type { MeResponse } from "@/client/types.gen";
import { useAuth, storeTokens } from "@/composables/useAuth";
import { generateOidcState } from "@/router";
import { API_BASE_URL } from "@/config";
import {
  Card, CardHeader, CardTitle, CardDescription, CardContent,
} from "@/components/ui/card";
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
const oidcLoading = ref(false);
const oidcAvailable = ref<boolean | null>(null); // null = checking

const redirect = computed(() =>
  typeof route.query.redirect === "string" ? route.query.redirect : "/packages",
);

// Already signed in → skip the login page.
onMounted(async () => {
  if (isAuthenticated.value) {
    router.replace(redirect.value);
    return;
  }
  // Probe whether the OIDC login endpoint is reachable.
  // Use GET (not HEAD) — actix-web doesn't automatically handle HEAD on GET routes.
  // state is omitted → server returns 200 (configured) or 503 (not configured).
  try {
    const res = await fetch(`${API_BASE_URL}/api/v1/auth/oidc/login`, {
      redirect: "manual",
    });
    oidcAvailable.value = res.ok;
  } catch {
    oidcAvailable.value = false;
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

function signInWithOidc() {
  oidcLoading.value = true;
  // Generate a random state, store in sessionStorage for CSRF validation,
  // then hand off to the backend which threads it through to the provider.
  const state = generateOidcState();
  window.location.href = `${API_BASE_URL}/api/v1/auth/oidc/login?state=${encodeURIComponent(state)}`;
}

function continueAnonymously() {
  const dest = redirect.value === "/login" ? "/packages" : redirect.value;
  router.push(dest);
}
</script>

<template>
  <div class="flex min-h-[calc(100vh-3.5rem)] items-center justify-center px-4">
    <Card class="w-full max-w-sm">
      <CardHeader class="space-y-1">
        <CardTitle class="text-2xl">Sign in</CardTitle>
        <CardDescription>
          Authenticate to access protected resources.
        </CardDescription>
      </CardHeader>

      <CardContent class="space-y-4">
        <!-- OIDC sign-in (shown once the probe resolves) -->
        <Button
          v-if="oidcAvailable"
          type="button"
          variant="outline"
          class="w-full"
          :disabled="oidcLoading"
          @click="signInWithOidc"
        >
          {{ oidcLoading ? "Redirecting…" : "Sign in with OIDC" }}
        </Button>

        <!-- Divider between OIDC and token form -->
        <div v-if="oidcAvailable" class="relative">
          <div class="absolute inset-0 flex items-center">
            <span class="w-full border-t" />
          </div>
          <div class="relative flex justify-center text-xs uppercase">
            <span class="bg-card px-2 text-muted-foreground">or use a token</span>
          </div>
        </div>

        <!-- Static-token form -->
        <form @submit.prevent="submit" class="space-y-4">
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

          <p v-if="error" class="text-sm text-destructive">{{ error }}</p>

          <Button type="submit" class="w-full" :disabled="loading">
            {{ loading ? "Signing in…" : "Sign in with token" }}
          </Button>
        </form>

        <Button
          type="button"
          variant="ghost"
          class="w-full"
          @click="continueAnonymously"
        >
          Continue without signing in
        </Button>
      </CardContent>
    </Card>
  </div>
</template>
