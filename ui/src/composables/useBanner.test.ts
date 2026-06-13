import { describe, it, expect, vi, afterEach } from "vitest";
import { withSetup } from "@/test/withSetup";
import { useBanner, type GlobalBanner } from "./useBanner";

describe("useBanner", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });

  it("fetches the banner on mount", async () => {
    const banner: GlobalBanner = { message: "hi", level: "info", set_at: "now", set_by: "admin" };
    const fetchMock = vi
      .fn()
      .mockResolvedValue(new Response(JSON.stringify(banner), { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);

    const [state] = withSetup(() => useBanner());
    await vi.waitFor(() => expect(state.banner.value).toEqual(banner));

    expect(fetchMock.mock.calls[0]?.[0]).toMatch(/\/api\/v1\/banner$/);
  });

  it("ignores network errors and leaves the banner null", async () => {
    const fetchMock = vi.fn().mockRejectedValue(new Error("network down"));
    vi.stubGlobal("fetch", fetchMock);

    const [state] = withSetup(() => useBanner());
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalled());

    expect(state.banner.value).toBeNull();
  });

  it("polls every 30s and stops after unmount", async () => {
    vi.useFakeTimers();
    const fetchMock = vi.fn().mockResolvedValue(new Response("null", { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);

    const [, app] = withSetup(() => useBanner());
    await vi.advanceTimersByTimeAsync(0);
    expect(fetchMock).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(30_000);
    expect(fetchMock).toHaveBeenCalledTimes(2);

    app.unmount();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });
});
