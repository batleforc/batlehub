import { describe, expect, it } from "vitest";

import {
  composite,
  contrastRatio,
  fromHex,
  isInGamut,
  maxChroma,
  oklchToLinearSrgb,
  parseOklch,
  relativeLuminance,
  toHex,
  type Rgb,
} from "./color.ts";

describe("oklchToLinearSrgb", () => {
  /** Anchors: pure black and pure white are exact in any correct implementation. */
  it("maps L=0 to black and L=1 to white", () => {
    expect(toHex(oklchToLinearSrgb(0, 0, 0))).toBe("#000000");
    expect(toHex(oklchToLinearSrgb(1, 0, 0))).toBe("#ffffff");
  });

  /** The value the token file ships, pinned so a matrix typo cannot pass. */
  it("converts the dark accent to its documented hex", () => {
    expect(toHex(oklchToLinearSrgb(0.65, 0.235, 25))).toBe("#ff343d");
  });

  it("converts the light accent to its documented hex", () => {
    expect(toHex(oklchToLinearSrgb(0.52, 0.21, 25))).toBe("#c50220");
  });
});

describe("isInGamut / maxChroma", () => {
  /**
   * Both of Monofolio's `--primary` values are outside sRGB. These are the two
   * cases that made The In-Gamut Rule law rather than a preference, so they are
   * pinned here as regression anchors.
   */
  it("rejects Monofolio's dark --primary and reports the limit", () => {
    expect(isInGamut(oklchToLinearSrgb(0.65, 0.26, 25))).toBe(false);
    expect(maxChroma(0.65, 25)).toBeCloseTo(0.2359, 3);
  });

  it("rejects Monofolio's light --primary and reports the limit", () => {
    expect(isInGamut(oklchToLinearSrgb(0.52, 0.24, 25))).toBe(false);
    expect(maxChroma(0.52, 25)).toBeCloseTo(0.2108, 3);
  });

  it("accepts the clamped values the tokens actually ship", () => {
    expect(isInGamut(oklchToLinearSrgb(0.65, 0.235, 25))).toBe(true);
    expect(isInGamut(oklchToLinearSrgb(0.52, 0.21, 25))).toBe(true);
  });

  it("a colour at exactly maxChroma is in gamut", () => {
    for (const [l, h] of [
      [0.65, 25],
      [0.5, 52],
      [0.85, 85],
    ]) {
      expect(isInGamut(oklchToLinearSrgb(l, maxChroma(l, h), h)), `L=${l} H=${h}`).toBe(true);
    }
  });

  it("chroma 0 is always in gamut", () => {
    for (const l of [0, 0.25, 0.5, 0.75, 1]) {
      expect(isInGamut(oklchToLinearSrgb(l, 0, 0))).toBe(true);
    }
  });
});

describe("contrastRatio", () => {
  it("is 21:1 for black on white and 1:1 for a colour on itself", () => {
    const black = oklchToLinearSrgb(0, 0, 0);
    const white = oklchToLinearSrgb(1, 0, 0);
    expect(contrastRatio(black, white)).toBeCloseTo(21, 5);
    expect(contrastRatio(white, white)).toBeCloseTo(1, 10);
  });

  it("is order-independent", () => {
    const a = oklchToLinearSrgb(0.07, 0.018, 18);
    const b = oklchToLinearSrgb(0.93, 0.018, 25);
    expect(contrastRatio(a, b)).toBeCloseTo(contrastRatio(b, a), 10);
  });

  /** The headline figure from DESIGN.md, so a regression is visible by name. */
  it("reproduces the documented ink-on-ground ratio", () => {
    const ground = oklchToLinearSrgb(0.07, 0.018, 18);
    const ink = oklchToLinearSrgb(0.93, 0.018, 25);
    expect(contrastRatio(ink, ground)).toBeCloseTo(16.88, 1);
  });
});

describe("relativeLuminance", () => {
  it("clamps out-of-gamut components rather than returning nonsense", () => {
    const lum = relativeLuminance(oklchToLinearSrgb(0.65, 0.26, 25));
    expect(lum).toBeGreaterThanOrEqual(0);
    expect(lum).toBeLessThanOrEqual(1);
  });
});

describe("parseOklch", () => {
  it("parses the plain form", () => {
    expect(parseOklch("oklch(0.65 0.235 25)")).toEqual({ l: 0.65, c: 0.235, h: 25, alpha: 1 });
  });

  it("parses percentage lightness and an alpha component", () => {
    expect(parseOklch("oklch(65% 0.236 25 / 50%)")).toEqual({
      l: 0.65,
      c: 0.236,
      h: 25,
      alpha: 0.5,
    });
  });

  it("tolerates surrounding whitespace", () => {
    expect(parseOklch("  oklch(0.5 0.1 20)  ")).not.toBeNull();
  });

  it("returns null for anything that is not an oklch() value", () => {
    for (const bad of ["#ff343d", "rgb(255 51 60)", "oklch()", "", "oklch(0.5 0.1)"]) {
      expect(parseOklch(bad), bad).toBeNull();
    }
  });
});

describe("fromHex", () => {
  it("round-trips through toHex", () => {
    for (const hex of ["#000000", "#ffffff", "#57606a", "#bdc4cc"]) {
      expect(toHex(fromHex(hex))).toBe(hex);
    }
  });

  it("expands the three-digit form", () => {
    expect(toHex(fromHex("#abc"))).toBe("#aabbcc");
  });
});

/**
 * `composite` is what twelve failing contrast assertions rest on, so it is
 * worth proving on its own rather than only through them.
 */
describe("composite", () => {
  const BLACK: Rgb = [0, 0, 0];
  const WHITE: Rgb = [1, 1, 1];

  it("returns the background at alpha 0 and the foreground at alpha 1", () => {
    expect(composite(WHITE, BLACK, 0)).toEqual(BLACK);
    expect(toHex(composite(WHITE, BLACK, 1))).toBe("#ffffff");
  });

  /**
   * The whole reason this function exists rather than a linear-space lerp.
   *
   * CSS simple alpha compositing operates on the *encoded* channels, so white
   * at 50% over black is mid-grey **by byte value** — `#7f7f7f`, half of 255
   * rounding down — which carries 21.4% of white's luminance. Mixing in linear
   * light instead gives `#bcbcbc` at 50% luminance: a visibly different colour,
   * and at the ratios this codebase measures, the difference between clearing
   * 4.5:1 and missing it.
   */
  it("mixes on the gamma-encoded channels, as a browser does", () => {
    expect(toHex(composite(WHITE, BLACK, 0.5))).toBe("#7f7f7f");
    expect(relativeLuminance(composite(WHITE, BLACK, 0.5))).toBeCloseTo(0.214, 3);

    // The linear-space answer, for contrast — not what any engine paints.
    expect(toHex([0.5, 0.5, 0.5])).toBe("#bcbcbc");
    expect(relativeLuminance([0.5, 0.5, 0.5])).toBeCloseTo(0.5, 3);
  });

  it("is symmetric in the pair when alpha is halved from either side", () => {
    // `a over b` at 0.5 and `b over a` at 0.5 are the same mixture.
    expect(toHex(composite(WHITE, BLACK, 0.5))).toBe(toHex(composite(BLACK, WHITE, 0.5)));
  });
});
