import { describe, it, expect, vi, afterEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";

import GlobalBanner from "./GlobalBanner.vue";
import type { GlobalBanner as Banner } from "@/composables/useBanner";

const banner = (over: Partial<Banner> = {}): Banner => ({
  message: "Scheduled maintenance at 22:00 UTC",
  level: "info",
  set_at: "2026-08-15T10:00:00Z",
  set_by: "admin",
  ...over,
});

async function mountBanner(payload: Banner | null) {
  vi.stubGlobal(
    "fetch",
    vi.fn(() => Promise.resolve(new Response(JSON.stringify(payload), { status: 200 }))),
  );
  const wrapper = mount(GlobalBanner);
  await flushPromises();
  return wrapper;
}

/**
 * The strip an admin uses to tell everyone something. Its only job beyond
 * showing the text is to make the severity readable *without* the text — three
 * levels, three icons, three colours — so an error never reads as an FYI.
 */
describe("GlobalBanner", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("renders nothing when there is no banner", async () => {
    const wrapper = await mountBanner(null);
    expect(wrapper.find("div").exists()).toBe(false);
  });

  it("shows the message once one is set", async () => {
    const wrapper = await mountBanner(banner());
    expect(wrapper.text()).toContain("Scheduled maintenance at 22:00 UTC");
  });

  it("gives each level its own icon and colour", async () => {
    const marks = await Promise.all(
      (["info", "warning", "error"] as const).map(async (level) => {
        const wrapper = await mountBanner(banner({ level }));
        const root = wrapper.get("div");
        return {
          classes: root.attributes("class"),
          icon: root.find("svg").html(),
        };
      }),
    );

    // Any two levels sharing a colour or an icon is the defect.
    expect(new Set(marks.map((m) => m.classes)).size).toBe(3);
    expect(new Set(marks.map((m) => m.icon)).size).toBe(3);
  });

  it("marks an error banner as destructive", async () => {
    const wrapper = await mountBanner(banner({ level: "error" }));
    expect(wrapper.get("div").attributes("class")).toContain("text-destructive");
  });

  it("marks a warning banner in copper", async () => {
    const wrapper = await mountBanner(banner({ level: "warning" }));
    expect(wrapper.get("div").attributes("class")).toContain("text-copper");
  });
});
