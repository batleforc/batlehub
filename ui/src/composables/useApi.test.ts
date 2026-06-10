import { describe, it, expect, vi } from "vitest";
import { withSetup } from "@/test/withSetup";
import { useApi, extractMessage } from "./useApi";

describe("extractMessage", () => {
  it("returns 'Unknown error' for null/undefined", () => {
    expect(extractMessage(null)).toBe("Unknown error");
    expect(extractMessage(undefined)).toBe("Unknown error");
  });

  it("returns string errors directly", () => {
    expect(extractMessage("boom")).toBe("boom");
  });

  it("returns Error.message", () => {
    expect(extractMessage(new Error("oops"))).toBe("oops");
  });

  it("extracts .message from a plain object", () => {
    expect(extractMessage({ message: "bad request" })).toBe("bad request");
  });

  it("extracts .error from a plain object", () => {
    expect(extractMessage({ error: "forbidden" })).toBe("forbidden");
  });

  it("falls back to String(err) for other types", () => {
    expect(extractMessage(42)).toBe("42");
  });
});

describe("useApi", () => {
  it("starts loading and then populates data on success", async () => {
    const fn = vi.fn().mockResolvedValue({ data: { id: 1 } });
    const [state] = withSetup(() => useApi<{ id: number }>(fn));

    expect(state.loading.value).toBe(true);

    await vi.waitFor(() => expect(state.loading.value).toBe(false));
    expect(state.data.value).toEqual({ id: 1 });
    expect(state.error.value).toBeNull();
  });

  it("captures an error returned by fn", async () => {
    const fn = vi.fn().mockResolvedValue({ error: { message: "nope" } });
    const [state] = withSetup(() => useApi(fn));

    await vi.waitFor(() => expect(state.loading.value).toBe(false));
    expect(state.error.value).toBe("nope");
    expect(state.data.value).toBeNull();
  });

  it("captures a thrown error", async () => {
    const fn = vi.fn().mockRejectedValue(new Error("network down"));
    const [state] = withSetup(() => useApi(fn));

    await vi.waitFor(() => expect(state.loading.value).toBe(false));
    expect(state.error.value).toBe("network down");
    expect(state.data.value).toBeNull();
  });

  it("reload() re-invokes fn", async () => {
    const fn = vi.fn().mockResolvedValue({ data: "ok" });
    const [state] = withSetup(() => useApi(fn));

    await vi.waitFor(() => expect(fn).toHaveBeenCalledTimes(1));

    state.reload();
    await vi.waitFor(() => expect(fn).toHaveBeenCalledTimes(2));
  });
});
