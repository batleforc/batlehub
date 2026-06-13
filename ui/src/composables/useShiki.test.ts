import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("shiki/engine/javascript", () => ({
  createJavaScriptRegexEngine: vi.fn(() => ({})),
}));

describe("useShiki", () => {
  beforeEach(() => {
    vi.resetModules();
  });

  it("highlight returns '' before the highlighter is ready", async () => {
    let resolveHighlighter: (h: unknown) => void = () => {};
    vi.doMock("shiki/core", () => ({
      createHighlighterCore: vi.fn(
        () =>
          new Promise((resolve) => {
            resolveHighlighter = resolve;
          }),
      ),
    }));

    const { useShiki } = await import("./useShiki");
    const { highlight, ready } = useShiki();

    expect(ready.value).toBe(false);
    expect(highlight("foo", "toml")).toBe("");

    // Resolve so the pending promise doesn't leak into other tests.
    resolveHighlighter({ codeToHtml: vi.fn() });
  });

  it("becomes ready and highlights code once the highlighter resolves", async () => {
    const codeToHtml = vi.fn(() => "<pre>foo = 1</pre>");
    vi.doMock("shiki/core", () => ({
      createHighlighterCore: vi.fn(() => Promise.resolve({ codeToHtml })),
    }));

    const { useShiki } = await import("./useShiki");
    const { highlight, ready } = useShiki();

    await vi.waitFor(() => expect(ready.value).toBe(true));

    expect(highlight("foo = 1", "toml")).toBe("<pre>foo = 1</pre>");
    expect(codeToHtml).toHaveBeenCalledWith("foo = 1", expect.objectContaining({ lang: "toml" }));
  });

  it("returns '' if codeToHtml throws", async () => {
    const codeToHtml = vi.fn(() => {
      throw new Error("boom");
    });
    vi.doMock("shiki/core", () => ({
      createHighlighterCore: vi.fn(() => Promise.resolve({ codeToHtml })),
    }));

    const { useShiki } = await import("./useShiki");
    const { highlight, ready } = useShiki();

    await vi.waitFor(() => expect(ready.value).toBe(true));

    expect(highlight("foo", "toml")).toBe("");
  });
});
