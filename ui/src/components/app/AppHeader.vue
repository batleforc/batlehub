<script setup lang="ts">
import { useI18n } from "vue-i18n";
import { computed } from "vue";
import { RouterLink, useRoute, useRouter } from "vue-router";
import { Menu, X, BookOpen, FolderKey } from "@lucide/vue";
import { useAuth } from "@/composables/useAuth";
import { primaryNav } from "@/config/navigation";
import { DOCS_URL } from "@/config";
import { Button } from "@/components/ui/button";
import ThemeToggle from "@/components/ThemeToggle.vue";
import LocaleToggle from "@/components/LocaleToggle.vue";
import AppNav from "./AppNav.vue";
import UserMenu from "./UserMenu.vue";
import GlobalBanner from "./GlobalBanner.vue";

const { t } = useI18n();

const mobileOpen = defineModel<boolean>("mobileOpen", { default: false });

const { identity, isAdmin, isAuthenticated, logout } = useAuth();
const route = useRoute();
const router = useRouter();

const isOidcUser = computed(() => isAuthenticated.value && !!identity.value?.auth_provider);

/* The bar is opaque and unblurred: DESIGN.md's Elevation rule leaves this
   system no blurs and no layered surfaces, and a translucent sticky bar is
   also how the nav labels were failing AA — measured at 1.3:1 against whatever
   happened to scroll under them. The hairline rule below separates the bar
   from the sheet; it does not need to be see-through to do that.

   The bar follows the viewer (RFC 0003 §4.1): an item appears when it can be
   used, so nothing here leads to a redirect. Diagnostics moved to /tools —
   they are linked from the errors that motivate them instead of holding
   primary-nav weight for the rest of the year.

   All of them go through one segment box, Admin included. Splitting Admin out
   behind a divider gave the same choice two different shapes and made the bar
   read as two bars; the proof's masthead runs Packages/Setup/Admin as three
   cells of one control, and Admin's copper ink is what marks it out (AppNav). */
const navLinks = computed(() =>
  primaryNav({
    isAuthenticated: isAuthenticated.value,
    isAdmin: isAdmin.value,
    hasRegistryAccess: identity.value?.has_registry_access !== false,
  }),
);

function isActive(to: string) {
  return route.path === to || route.path.startsWith(to + "/");
}

function handleLogout() {
  logout();
  router.push("/login");
  mobileOpen.value = false;
}
</script>

<template>
  <header class="sticky top-0 z-40 border-b border-rule-soft bg-ground-sunk">
    <!-- The instance's announcement, above the masthead rather than under it.

         It sat below the nav, which put a maintenance or outage notice *third*
         in the reading order — after the wordmark and the whole primary bar —
         for something whose entire job is to be read before anything else on
         the page. It stays inside the sticky header on purpose: an error-level
         banner that scrolls away is a banner an operator can miss by scrolling,
         and moving it out to `App.vue` would have bought "topmost" at the cost
         of "always visible". Here it is both. -->
    <GlobalBanner />

    <!-- Full-bleed, not a centred container. The proof's `.masthead` is a body
         child with `max-width:100%` and `padding: --s3 --s5`, so the wordmark
         is flush to the left edge and the controls to the right: a bar that
         spans the window edge to edge but whose *contents* stop 260px short of
         it on a wide screen is drawing a frame it does not fill. Edge padding
         steps 24 → 16 at the same breakpoint the bar already reorganises at,
         so there is one boundary in this component rather than two (the proof
         breaks at 900, between our md and lg; md is where our nav collapses). -->
    <div class="flex h-14 items-center gap-3 px-4 md:gap-6 md:px-6">
      <!-- The wordmark goes home. It pointed at /packages from when `/` was a
           blind redirect there; Phase 4 made `/` a real identity-aware surface
           (RFC 0003 §4.3), so the logo had been skipping past it ever since.

           The mark is the proof's own: a 3×3 dot matrix, which is the display
           face's pixel drawn at logo scale rather than a stock parcel icon from
           a different drawing system. Set in Silkscreen at the Pixel-md step,
           ink not crimson — the accent is one dot, spent where it terminates
           the word, so the bar holds a single saturated pixel instead of a
           saturated line of type. -->
      <RouterLink
        to="/"
        class="flex items-center gap-3 shrink-0 font-display text-sub leading-none tracking-[0.04em] text-foreground transition-colors hover:text-copper sm:text-xl"
      >
        <svg
          width="16"
          height="16"
          viewBox="0 0 16 16"
          aria-hidden="true"
          fill="currentColor"
          class="shrink-0"
        >
          <rect x="0" y="0" width="4" height="4" />
          <rect x="6" y="0" width="4" height="4" />
          <rect x="12" y="0" width="4" height="4" />
          <rect x="0" y="6" width="4" height="4" />
          <rect x="6" y="6" width="4" height="4" />
          <rect x="12" y="6" width="4" height="4" />
          <rect x="0" y="12" width="4" height="4" />
          <rect x="6" y="12" width="4" height="4" />
          <rect x="12" y="12" width="4" height="4" />
        </svg>
        <span>BatleHub<span class="text-primary">.</span></span>
      </RouterLink>

      <AppNav :links="navLinks" variant="desktop" />

      <!-- The proof's textured spacer: dot-matrix as a *material*, at the same
           9px pitch as the board's texture swatch, living in the one box of the
           bar that holds no text and so cannot lose contrast to it. It is also
           the flex spacer, which means it collapses to nothing exactly when the
           bar gets tight — the texture is never what survives a narrow
           viewport. Masked to fade at both edges so it reads as a field the bar
           passes through rather than a panel with borders. -->
      <div class="relative hidden self-stretch md:block md:flex-1" aria-hidden="true">
        <span
          class="pointer-events-none absolute inset-0 opacity-55 [background-image:radial-gradient(var(--rule-soft)_1px,transparent_1.5px)] [background-size:9px_9px] [mask-image:linear-gradient(90deg,transparent,#000_40%,transparent)]"
        />
      </div>

      <!-- Right side -->
      <div class="ml-auto flex items-center gap-2 md:ml-0">
        <!-- The proof's `.ctl`: a bordered uppercase cell, the same edge weight
             as the nav segment, so a control and a destination are told apart
             by shape rather than by two unrelated shapes both being blue-ish. -->
        <a
          :href="DOCS_URL"
          target="_blank"
          rel="noopener noreferrer"
          class="hidden sm:flex items-center gap-2 whitespace-nowrap border border-border px-3 py-2 text-xs uppercase tracking-[0.1em] text-muted-foreground transition-colors hover:border-muted-foreground hover:text-foreground"
        >
          <BookOpen class="h-3.5 w-3.5" />
          {{ t("common.docs") }}
        </a>
        <LocaleToggle />
        <ThemeToggle />

        <UserMenu />

        <!-- Mobile menu toggle -->
        <Button
          variant="ghost"
          size="icon"
          class="md:hidden"
          :aria-label="mobileOpen ? t('a11y.closeMenu') : t('a11y.openMenu')"
          :aria-expanded="mobileOpen"
          aria-controls="mobile-nav"
          @click="mobileOpen = !mobileOpen"
        >
          <X v-if="mobileOpen" class="h-4 w-4" />
          <Menu v-else class="h-4 w-4" />
        </Button>
      </div>
    </div>

    <!-- Mobile nav.

         One ruled run, not a stack of floating pills: the segment grammar the
         desktop bar uses, turned on its side. Every row is a cell of the same
         box and the chosen one reverses to ink, so "where am I" is answered the
         same way at both widths. -->
    <div
      v-if="mobileOpen"
      id="mobile-nav"
      class="md:hidden border-t border-rule-soft bg-ground-sunk px-4 py-4"
    >
      <nav :aria-label="t('nav.aria')" class="border border-border divide-y divide-border">
        <AppNav :links="navLinks" variant="mobile" @navigate="mobileOpen = false" />
        <RouterLink
          v-if="isOidcUser"
          to="/me/tokens"
          :aria-current="isActive('/me/tokens') ? 'page' : undefined"
          :class="[
            'block px-3 py-2.5 text-xs uppercase tracking-[0.1em] transition-colors',
            isActive('/me/tokens')
              ? 'bg-foreground text-background font-bold'
              : 'text-muted-foreground hover:text-foreground',
          ]"
          @click="mobileOpen = false"
          >{{ t("appHeader.myTokens") }}</RouterLink
        >
        <RouterLink
          v-if="isAuthenticated"
          to="/me/profile"
          :aria-current="isActive('/me/profile') ? 'page' : undefined"
          :class="[
            'block px-3 py-2.5 text-xs uppercase tracking-[0.1em] transition-colors',
            isActive('/me/profile')
              ? 'bg-foreground text-background font-bold'
              : 'text-muted-foreground hover:text-foreground',
          ]"
          @click="mobileOpen = false"
          >{{ t("appHeader.myProfile") }}</RouterLink
        >
        <RouterLink
          v-if="isAuthenticated"
          to="/me/namespace"
          :aria-current="isActive('/me/namespace') ? 'page' : undefined"
          :class="[
            'flex items-center gap-2 px-3 py-2.5 text-xs uppercase tracking-[0.1em] transition-colors',
            isActive('/me/namespace')
              ? 'bg-foreground text-background font-bold'
              : 'text-muted-foreground hover:text-foreground',
          ]"
          @click="mobileOpen = false"
        >
          <FolderKey class="h-3.5 w-3.5" />
          {{ t("appHeader.myNamespace") }}
        </RouterLink>
        <a
          :href="DOCS_URL"
          target="_blank"
          rel="noopener noreferrer"
          class="flex items-center gap-2 px-3 py-2.5 text-xs uppercase tracking-[0.1em] text-muted-foreground hover:text-foreground transition-colors"
          @click="mobileOpen = false"
        >
          <BookOpen class="h-3.5 w-3.5" />
          {{ t("common.documentation") }}
        </a>
      </nav>
      <button
        v-if="isAuthenticated"
        class="mt-3 block w-full border border-border px-3 py-2.5 text-left text-xs uppercase tracking-[0.1em] text-destructive transition-colors hover:border-destructive"
        @click="handleLogout"
      >
        {{ t("appHeader.signOut") }}
      </button>
    </div>
  </header>
</template>
