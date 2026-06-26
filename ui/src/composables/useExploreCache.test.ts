import { describe, it, expect, beforeEach, vi, afterEach } from "vitest";
import { useExploreCache } from "./useExploreCache";

describe("useExploreCache", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    // Reset the module-level cache between tests by invalidating all entries.
    const { invalidate } = useExploreCache<unknown>();
    invalidate();
  });

  it("returns undefined for a key that has not been set", () => {
    const { get } = useExploreCache<string[]>();
    expect(get("npm", 0, "name", "react")).toBeUndefined();
  });

  it("returns the stored value immediately after set", () => {
    const { get, set } = useExploreCache<string[]>();
    set("npm", 0, "name", "react", ["react-18"]);
    expect(get("npm", 0, "name", "react")).toEqual(["react-18"]);
  });

  it("different keys are independent", () => {
    const { get, set } = useExploreCache<number>();
    set("npm", 0, "name", "react", 1);
    set("cargo", 0, "name", "serde", 2);
    expect(get("npm", 0, "name", "react")).toBe(1);
    expect(get("cargo", 0, "name", "serde")).toBe(2);
  });

  it("returns undefined after TTL has expired", () => {
    const { get, set } = useExploreCache<string>();
    set("npm", 0, "name", "", "data");
    // Advance past the 5-minute TTL.
    vi.advanceTimersByTime(5 * 60 * 1_000 + 1);
    expect(get("npm", 0, "name", "")).toBeUndefined();
  });

  it("value is still present just before TTL expires", () => {
    const { get, set } = useExploreCache<string>();
    set("npm", 0, "name", "", "data");
    vi.advanceTimersByTime(5 * 60 * 1_000 - 1);
    expect(get("npm", 0, "name", "")).toBe("data");
  });

  it("invalidate() with a registry clears only that registry's entries", () => {
    const { get, set, invalidate } = useExploreCache<string>();
    set("npm", 0, "name", "", "npm-data");
    set("cargo", 0, "name", "", "cargo-data");
    invalidate("npm");
    expect(get("npm", 0, "name", "")).toBeUndefined();
    expect(get("cargo", 0, "name", "")).toBe("cargo-data");
  });

  it("invalidate() without arguments clears everything", () => {
    const { get, set, invalidate } = useExploreCache<string>();
    set("npm", 0, "name", "", "a");
    set("cargo", 0, "name", "", "b");
    invalidate();
    expect(get("npm", 0, "name", "")).toBeUndefined();
    expect(get("cargo", 0, "name", "")).toBeUndefined();
  });

  it("set overwrites an existing entry with the same key", () => {
    const { get, set } = useExploreCache<string>();
    set("npm", 0, "name", "", "v1");
    set("npm", 0, "name", "", "v2");
    expect(get("npm", 0, "name", "")).toBe("v2");
  });

  it("overwriting resets the TTL", () => {
    const { get, set } = useExploreCache<string>();
    // Set entry, advance 4 minutes, re-set it, advance 4 more minutes (8 total).
    set("npm", 0, "name", "", "v1");
    vi.advanceTimersByTime(4 * 60 * 1_000);
    set("npm", 0, "name", "", "v2");
    vi.advanceTimersByTime(4 * 60 * 1_000);
    // 4 min past the second set — should still be valid (TTL = 5 min).
    expect(get("npm", 0, "name", "")).toBe("v2");
  });
});
