import { onMounted, onUnmounted, ref, type Ref } from "vue";

export interface GlobalBanner {
  message: string;
  level: "info" | "warning" | "error";
  set_at: string;
  set_by: string;
}

const POLL_INTERVAL_MS = 30_000;
const API_BASE =
  (import.meta as unknown as { env: Record<string, string> }).env.VITE_API_BASE_URL ?? "";

export function useBanner(): { banner: Ref<GlobalBanner | null> } {
  const banner = ref<GlobalBanner | null>(null);
  let timer: ReturnType<typeof setInterval> | null = null;

  async function fetchBanner() {
    try {
      const resp = await fetch(`${API_BASE}/api/v1/banner`);
      if (resp.ok) {
        banner.value = (await resp.json()) as GlobalBanner | null;
      }
    } catch {
      // ignore network errors — the banner is non-critical
    }
  }

  onMounted(() => {
    void fetchBanner();
    timer = setInterval(() => void fetchBanner(), POLL_INTERVAL_MS);
  });

  onUnmounted(() => {
    if (timer !== null) clearInterval(timer);
  });

  return { banner };
}
