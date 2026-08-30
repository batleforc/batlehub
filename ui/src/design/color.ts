/**
 * OKLCH → sRGB conversion and WCAG contrast, with no dependencies.
 *
 * This exists so the design system's colour rules can be *enforced* rather than
 * asserted in prose. Two of them are only checkable with real arithmetic:
 *
 *  - **The In-Gamut Rule** — every token must be in-gamut for sRGB as authored.
 *    A token outside the gamut cannot carry a contrast guarantee, because
 *    engines disagree on what they paint: naive clipping and CSS Color 4 chroma
 *    reduction land on different colours with different ratios. Two of
 *    Monofolio's own `--primary` values turned out to be outside it.
 *  - **AA floors** — the ratios quoted in DESIGN.md have to stay true as tokens
 *    are edited, in both renditions, or they are decoration.
 *
 * Matrices are the standard Björn Ottosson OKLab constants.
 */

export type Rgb = readonly [number, number, number];

/** OKLCH → *linear* sRGB. Values may fall outside [0,1] when out of gamut. */
export function oklchToLinearSrgb(l: number, c: number, h: number): Rgb {
  const hRad = (h * Math.PI) / 180;
  const a = c * Math.cos(hRad);
  const b = c * Math.sin(hRad);

  const lp = l + 0.3963377774 * a + 0.2158037573 * b;
  const mp = l - 0.1055613458 * a - 0.0638541728 * b;
  const sp = l - 0.0894841775 * a - 1.291485548 * b;

  const l3 = lp ** 3;
  const m3 = mp ** 3;
  const s3 = sp ** 3;

  return [
    4.0767416621 * l3 - 3.3077115913 * m3 + 0.2309699292 * s3,
    -1.2684380046 * l3 + 2.6097574011 * m3 - 0.3413193965 * s3,
    -0.0041960863 * l3 - 0.7034186147 * m3 + 1.707614701 * s3,
  ];
}

const EPSILON = 1e-6;

/** Whether a colour is representable in sRGB without clipping. */
export function isInGamut(rgb: Rgb): boolean {
  return rgb.every((v) => v >= -EPSILON && v <= 1 + EPSILON);
}

/**
 * The largest chroma that stays in sRGB at a given lightness and hue.
 *
 * Binary search rather than a closed form: the gamut boundary in OKLCH has no
 * simple analytic expression, and 60 iterations is exact to well beyond the
 * precision anyone authors a token at.
 */
export function maxChroma(l: number, h: number): number {
  let lo = 0;
  let hi = 0.5;
  for (let i = 0; i < 60; i++) {
    const mid = (lo + hi) / 2;
    if (isInGamut(oklchToLinearSrgb(l, mid, h))) lo = mid;
    else hi = mid;
  }
  return lo;
}

/** WCAG 2.x relative luminance. Clamps, so an out-of-gamut colour still scores. */
export function relativeLuminance(rgb: Rgb): number {
  const [r, g, b] = rgb.map((v) => Math.min(1, Math.max(0, v)));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG 2.x contrast ratio, 1–21. Order-independent. */
export function contrastRatio(a: Rgb, b: Rgb): number {
  const [hi, lo] = [relativeLuminance(a), relativeLuminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

const clamp = (v: number) => Math.min(1, Math.max(0, v));

/** Linear → gamma-encoded sRGB, one channel. */
const encodeSrgb = (v: number) =>
  v <= 0.0031308 ? 12.92 * v : 1.055 * Math.pow(v, 1 / 2.4) - 0.055;

/** Gamma-encoded sRGB → linear, one channel. */
const decodeSrgb = (v: number) => (v <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4));

/**
 * `fg` at `alpha` over `bg`, as a browser composites it.
 *
 * The gap this closes: every ratio in DESIGN.md is measured on tokens at full
 * opacity, and the component layer then paints them through an alpha —
 * `text-muted-foreground/60`, `border-primary/40`, `bg-primary/85`. What the
 * reader sees is the *composite*, which is a different colour with a different
 * ratio, and nothing measured it. Nine pairings turned out to sit under their
 * floor while `tokens.test.ts` was green and correct about every number in it.
 *
 * The mix happens on the **gamma-encoded** channels, not in linear light.
 * Compositing in linear space is arguably the more correct rendering and it is
 * emphatically not what CSS does: `simple alpha compositing` in the Compositing
 * spec operates on the encoded values. Mixing in linear light puts
 * `--ink-dim/60` on `--ground` at 3.17:1 instead of 2.57:1 — a difference that
 * spans the AA floor, in the direction that would have hidden the defect.
 *
 * In and out in linear sRGB, so the result feeds `contrastRatio` directly.
 */
export function composite(fg: Rgb, bg: Rgb, alpha: number): Rgb {
  const mix = (f: number, b: number) =>
    decodeSrgb(encodeSrgb(clamp(f)) * alpha + encodeSrgb(clamp(b)) * (1 - alpha));
  return [mix(fg[0], bg[0]), mix(fg[1], bg[1]), mix(fg[2], bg[2])];
}

/** `#rgb` / `#rrggbb` → linear sRGB, for the colours that arrive as hex. */
export function fromHex(hex: string): Rgb {
  const raw = hex.replace("#", "");
  const full = raw.length === 3 ? [...raw].map((ch) => ch + ch).join("") : raw;
  const byte = (i: number) => Number.parseInt(full.slice(i * 2, i * 2 + 2), 16) / 255;
  return [decodeSrgb(byte(0)), decodeSrgb(byte(1)), decodeSrgb(byte(2))];
}

/** Gamma-encoded hex, for reporting. Clamps out-of-gamut components. */
export function toHex(rgb: Rgb): string {
  return (
    "#" +
    rgb
      .map((v) => {
        const byte = Math.round(Math.min(255, Math.max(0, encodeSrgb(clamp(v)) * 255)));
        return byte.toString(16).padStart(2, "0");
      })
      .join("")
  );
}

export interface Oklch {
  l: number;
  c: number;
  h: number;
  /** Alpha, when the value carried a `/ a` component. */
  alpha: number;
}

/** A CSS numeric token, bare or as a percentage where the grammar allows one. */
const NUM = String.raw`[\d.]+`;

/**
 * `oklch(L C H)` and `oklch(L C H / A)`.
 *
 * Assembled from `NUM` rather than written out: spelled inline, the same
 * character class appears four times and the pattern reads as punctuation.
 */
const OKLCH = new RegExp(
  String.raw`^oklch\(\s*(${NUM}%?)\s+(${NUM})\s+(${NUM})\s*(?:/\s*(${NUM}%?)\s*)?\)$`,
  "i",
);

/**
 * Parse `oklch(L C H)` / `oklch(L C H / A)`, accepting the percentage form for
 * lightness and alpha that CSS also allows.
 */
export function parseOklch(value: string): Oklch | null {
  const match = OKLCH.exec(value.trim());
  if (!match) return null;

  const num = (raw: string): number =>
    raw.endsWith("%") ? Number.parseFloat(raw) / 100 : Number.parseFloat(raw);

  return {
    l: num(match[1]),
    c: Number.parseFloat(match[2]),
    h: Number.parseFloat(match[3]),
    alpha: match[4] === undefined ? 1 : num(match[4]),
  };
}
