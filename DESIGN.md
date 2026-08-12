---
name: BatleHub
description: A dark bitmap type specimen for an artifact proxy, where an artifact's state is its resolution.
colors:
  ground: "oklch(0.07 0.018 18)"
  ground-raised: "oklch(0.155 0.022 18)"
  ground-sunk: "oklch(0.045 0.014 18)"
  ink: "oklch(0.93 0.018 25)"
  ink-dim: "oklch(0.62 0.045 30)"
  rule-soft: "oklch(0.34 0.03 25)"
  rule-strong: "oklch(0.52 0.06 25)"
  accent: "oklch(0.65 0.236 25)"
  accent-ink: "oklch(0.10 0.02 18)"
  copper: "oklch(0.72 0.14 52)"
  focus: "oklch(0.85 0.16 85)"
typography:
  display:
    fontFamily: "Silkscreen, monospace"
    fontSize: "56px"
    fontWeight: 700
    lineHeight: 0.92
    letterSpacing: "0.02em"
  pixel-md:
    fontFamily: "Silkscreen, monospace"
    fontSize: "24px"
    fontWeight: 700
    lineHeight: 1.6
    letterSpacing: "0.04em"
  pixel-sm:
    fontFamily: "Silkscreen, monospace"
    fontSize: "16px"
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: "0.04em"
  head:
    fontFamily: "JetBrains Mono, ui-monospace, monospace"
    fontSize: "20px"
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: "normal"
  sub:
    fontFamily: "JetBrains Mono, ui-monospace, monospace"
    fontSize: "16px"
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: "normal"
  row:
    fontFamily: "JetBrains Mono, ui-monospace, monospace"
    fontSize: "15px"
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: "normal"
  body:
    fontFamily: "JetBrains Mono, ui-monospace, monospace"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: "normal"
  meta:
    fontFamily: "JetBrains Mono, ui-monospace, monospace"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.6
    letterSpacing: "0.1em"
rounded:
  none: "0px"
spacing:
  s1: "4px"
  s2: "8px"
  s3: "12px"
  s4: "16px"
  s5: "24px"
  s6: "40px"
components:
  button-action:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-ink}"
    typography: "{typography.pixel-sm}"
    rounded: "{rounded.none}"
    padding: "12px 16px"
  button-action-disabled:
    backgroundColor: "transparent"
    textColor: "{colors.ink-dim}"
    typography: "{typography.pixel-sm}"
    rounded: "{rounded.none}"
    padding: "12px 16px"
  button-ctl:
    backgroundColor: "transparent"
    textColor: "{colors.ink-dim}"
    typography: "{typography.meta}"
    rounded: "{rounded.none}"
    padding: "8px 12px"
  button-ctl-hover:
    backgroundColor: "transparent"
    textColor: "{colors.ink}"
    typography: "{typography.meta}"
    rounded: "{rounded.none}"
    padding: "8px 12px"
  input-search:
    backgroundColor: "{colors.ground-sunk}"
    textColor: "{colors.ink}"
    typography: "{typography.row}"
    rounded: "{rounded.none}"
    padding: "12px"
  nav-link:
    backgroundColor: "transparent"
    textColor: "{colors.ink-dim}"
    typography: "{typography.meta}"
    rounded: "{rounded.none}"
    padding: "8px 12px"
  nav-link-current:
    backgroundColor: "{colors.ink}"
    textColor: "{colors.ground}"
    typography: "{typography.meta}"
    rounded: "{rounded.none}"
    padding: "8px 12px"
  bay-item:
    backgroundColor: "transparent"
    textColor: "{colors.ink-dim}"
    typography: "{typography.row}"
    rounded: "{rounded.none}"
    padding: "8px 24px"
  bay-item-current:
    backgroundColor: "{colors.ground-raised}"
    textColor: "{colors.ink}"
    typography: "{typography.row}"
    rounded: "{rounded.none}"
    padding: "8px 24px"
  table-cell:
    backgroundColor: "transparent"
    textColor: "{colors.ink}"
    typography: "{typography.row}"
    rounded: "{rounded.none}"
    padding: "12px 12px 12px 0"
  table-row-hover:
    backgroundColor: "{colors.ground-raised}"
    textColor: "{colors.ink}"
    typography: "{typography.row}"
    rounded: "{rounded.none}"
    padding: "12px 12px 12px 0"
  panel:
    backgroundColor: "transparent"
    textColor: "{colors.ink-dim}"
    typography: "{typography.body}"
    rounded: "{rounded.none}"
    padding: "40px 24px"
  panel-error:
    backgroundColor: "transparent"
    textColor: "{colors.accent}"
    typography: "{typography.body}"
    rounded: "{rounded.none}"
    padding: "40px 24px"
---

# Design System: BatleHub

Recorded from the built proving surface at `ui/design-proof/index.html` (plus its authored
`halftone-plate.png` and self-hosted faces). That build is the ground truth for every value below;
where it diverges from the RFC's earlier intent, the build wins and the divergence is noted.
`ui/src` has not been migrated yet — Phase 2 derives `ui/src/design/tokens.css` from this file.

**Nothing here has been verified against a real render.** No browser can execute in the build
container. Unverified, and to be checked the first time this world is loaded: the mirrored plate as
composited, the gamut-mapped accent as painted, table reflow at 320–480px under the longer French
labels, focus rings as painted, whether the `woff2` files decode, and whether `filter: blur()` on
`<tr>` animates cleanly. Contrast ratios below were computed, not sampled.

## Overview

**Creative North Star: "The Proof Sheet"**

A press proof, pulled in a dark room. One form is set at poster scale and captioned by its facts;
everything under it is ruled rows, hairlines and ink. The world is an Emigre bitmap type specimen in
its dark rendition: a square-pixel display face carrying identity and scale, a mono text face
carrying every fact, and an authored halftone plate as the only image on the page. There is no
photography, no illustration, no card, no tile, and no glow.

The organising idea is **resolution as state**. What BatleHub holds and has verified renders at full
resolution — a fine 3×3 dot matrix, ink-white; what it does not renders coarse — a 2×2 matrix at
larger cells, in copper, dim or crimson. The single authored motion in the system is a row resolving
from blurred and low-contrast to crisp, which is what a cache miss becoming a hit actually looks
like. State is never carried by hue alone: pattern, word and hue all say it.

The ground is near-black because the confirmed use scene is a shell in a dark room, one tab among
twenty, often left on a second screen as ambient status — not because dark is the category default.
That scene also sets the density: this is a scanning surface, tuned for a reader who arrived with a
question after a build failed, and it earns its calm by refusing decoration rather than by adding
whitespace. The palette is the Monofolio OKLCH line, kept because measurement said it was also the
right palette for this world.

**Key Characteristics:**

- Near-black warm ground; one crimson accent, copper second; everything else is ink and rule.
- Two type ramps: a square-pixel display face on an 8px grid, and a mono text face for all data.
- Hairline rules, dashed at rest and solid under state; no cards, no tiles, no radius, no shadow at rest.
- State reads as resolution — fine dots for held-and-verified, coarse dots for not.
- Every denial states its own rule, in its own row, tied to the artifact it refused.
- One authored raster (a 60 lpi halftone plate) is the only image in the system.

## Colors

A single warm near-black ground carrying warm off-white ink, one synthetic crimson used sparingly,
and a copper second voice for anything that is *waiting* rather than *refused*.

### Primary

- **Signal Crimson** — `--accent`, the one synthetic colour in the world (`#ff333c` as painted).
  **5.76:1** on ground. Load-bearing exactly four ways and no more: link text, the fill under dark
  ink on the one primary action, the `blocked` state, and the 1px lit edge on the selected registry.
  Its dark counter-ink `--accent-ink` measures **5.69:1** on that fill — the correction of an
  incumbent fault, see the Counter-Ink Rule.
- **Counter-Ink** — `--accent-ink`, near-black at chroma 0.02. Used only as the foreground on a
  crimson fill, and as the near edge of the pixel step on `:active`. Never a background.

### Secondary

- **Aged Copper** — `--copper`, **8.06:1** on ground. The second voice, and it means *pending or
  held*, never *good*: the `stale` and `held` states, the instance hostname in the identity strip,
  the pressed border on a toggled control, the "synthetic fixture data" tag, and the plate's ink.
  It is the only colour that appears as a large area anywhere in the system, and only through the
  plate at 34% opacity.

### Tertiary

- **Signal Amber** — `--focus`, **13.07:1** on ground. Reserved entirely for the focus ring
  (`2px solid`, `2px` offset). It appears nowhere else, at no other size, for no other reason.

### Neutral

- **Ground** — `--ground`, the page. Warm near-black (L 0.07, hue 18), not neutral black.
- **Ground Sunk** — `--ground-sunk`, the masthead, the search field's well and the fixture-data
  footer. Computes to **1.005:1** against ground.
- **Ground Raised** — `--ground-raised`, row hover and the selected registry cell. Computes to
  **1.06:1** against ground.
- **Ink** — `--ink`, **16.88:1**. Package names, headings, display type, and the reversed foreground
  inside the active nav cell.
- **Dim Ink** — `--ink-dim`, **5.62:1** on ground and **5.28:1** on raised. Every label, column head,
  caption, count, timestamp and secondary control. It is the floor for small text; nothing smaller
  than 15px is ever painted in a lighter value than this.
- **Soft Rule** — `--rule-soft`. Separators only — section edges, the dashed row divider, the dot
  field. Never a control edge; it is not a contrast-carrying value.
- **Strong Rule** — `--rule-strong`, **3.68:1** on ground, **3.46:1** on raised, **3.70:1** on sunk.
  Every interactive boundary: control borders, the nav frame, the table head rule, the hovered row's
  divider, and the note's tie bar.

### Named Rules

**The In-Gamut Rule.** Every colour token is authored in-gamut for sRGB *as written*, never
caveated in prose. `--accent` is specified at max in-gamut chroma for its lightness and hue
(0.2359 at L 0.65 / H 25). Monofolio's source value `oklch(0.65 0.26 25)` is outside sRGB — engines
then disagree on what they paint, and a token that leaves the gamut cannot carry a contrast
guarantee. The Monofolio value is kept as a provenance comment, never as a computed value. If
wide-gamut is wanted later it layers as an `@supports (color: color(display-p3 …))` enhancement over
the in-gamut base, with AA measured against the base.

**The Undependable Fill Rule.** At this lightness a WCAG ratio is dominated by its own constant, so
no fill step separates two surfaces: `--ground-raised` is 1.06:1 against ground and `--ground-sunk`
is 1.005:1. Neither is a dependable surface, and neither may ever be the *only* signal for a state or
a boundary. State is carried by rule weight, ink and the dot field; fill is a secondary cue that
confirms what another channel already said. Both tokens stay declared — they do real work as
confirmation — but they are not elevation.

**The Counter-Ink Rule.** A crimson fill always takes `--accent-ink`, never white or light ink. The
incumbent `--primary-foreground` on `--primary` measured 3.58:1 and failed AA on every filled button
in the current UI; the corrected pairing measures 5.69:1. Do not re-inherit the light pairing when
porting a Monofolio component.

**The One Synthetic Rule.** Crimson is the world's only invented colour and stays on its four jobs.
Copper carries "waiting"; ink carries "known"; dim ink carries everything ordinary. A fifth hue
does not get added to signal a fifth condition — the dot pattern does that.

## Typography

**Display Font:** Silkscreen 400/700 (self-hosted `woff2`, fallback `monospace`)
**Body / Data Font:** JetBrains Mono, variable weight 100–800 (self-hosted `woff2`, fallback
`ui-monospace, monospace`)

**Character:** A square-pixel bitmap face for identity and scale against a precise, humane mono for
every fact on the page. The pairing is a specimen sheet's: one form shown large enough to see how it
is drawn, and a caption set small enough to be read. Both faces are self-hosted and preloaded, with
`font-display: swap` — a blank masthead on a cold cache is worse than a reflow.

**There are two ramps because Silkscreen is drawn on an 8px em.** Its square pixel only stays square
at integer multiples of 8, so every Silkscreen size is one: 16 / 24 / 56 / 72 / 88 / 104 px
(2× / 3× / 7× / 9× / 11× / 13×). JetBrains Mono carries no such constraint and uses a conventional
20 / 16 / 15 / 13 / 12 ramp.

### Hierarchy

- **Display** (Silkscreen 700, 56px → 72 → 88 → 104 discrete per breakpoint, line-height 0.92,
  0.02em, uppercase): the one form set at poster scale — the registry name at the top of the sheet.
  One per view.
- **Pixel Medium** (Silkscreen 700, 24px, 0.04em): the `BatleHub.` wordmark only.
- **Pixel Small** (Silkscreen, 16px): pixel-scale labels inside components — the registry name in a
  list row (0.02em), the primary action's label (0.04em, uppercase), a panel heading, a page number.
  This is the size at which the bitmap face is still a label and not yet an image.
- **Head** (JetBrains Mono, 20px) and **Sub** (16px): declared in the ramp but not exercised by this
  surface. Phase 2 inherits them as the section-title steps; treat their usage as unset, not as
  established.
- **Row** (JetBrains Mono, 15px, line-height 1.6): the scanning size — package names, list rows,
  search input. The densest text a reader is expected to read every line of.
- **Body** (JetBrains Mono, 13px, line-height 1.6): prose — the specimen caption (max 72ch), denial
  notes, panel copy (max 64ch), the pager.
- **Meta** (JetBrains Mono, 12px, uppercase, tracked): labels and chrome — column heads, nav items,
  state words, counts, controls, the identity strip. Always `--ink-dim` or better; never below 12px.

### Named Rules

**The Integer Em Rule.** Silkscreen only ever appears at a multiple of 8px. `--t-display` is
discrete per breakpoint (640 / 880 / 1140px) rather than a `clamp()` for exactly this reason: a fluid
10vw is an integer multiple at only a handful of viewport widths, which left the focal element off
its own pixel grid across the whole 560–1040px band. Never fluid-size the bitmap face.

**The Tracking Ladder Rule.** Uppercase mono is tracked, and the amount encodes how far the label is
from its content: 0.10em for inline chrome and state words, 0.14em for table column heads, 0.16em
for a standalone section label. Lowercase text is never tracked.

**The Data Face Rule.** Every number a reader might compare — counts, sizes, versions, page numbers
— is `font-variant-numeric: tabular-nums`, right-aligned when it sits in a column, and formatted
through one locale formatter shared by the whole surface. Never letterspace a number.

## Layout

**The sheet.** Full-bleed; there is no centred max-width container. The page is a masthead, a
specimen head, and then a two-column sheet: a 232px registry bay (sticky to the top of the viewport)
and a fluid catalog column that is allowed to shrink (`min-width: 0`) so the table controls its own
overflow.

**Rhythm.** One 4px-based scale, six steps: 4 / 8 / 12 / 16 / 24 / 40. 12px is the inside of a
control, 24px the inside of a region, 40px the top of the specimen and the inside of an empty-state
panel. There is no 32px step and no half-steps; a value not on the scale is a defect.

**Measure.** Prose is capped even though the sheet is not: the specimen caption at 72ch, panel copy
at 64ch. Table cells are unconstrained but package names wrap on any character (`word-break`),
because the names are `@scope/pkg` and `org.springframework:spring-core`, not sentences.

**Responsive.** Four breakpoints, and only one of them changes structure:

- **640 / 880 / 1140px** — display size steps only (72 / 88 / 104px). Nothing else moves.
- **900px and below** — the sheet collapses to one column; the bay unsticks and becomes a
  horizontally scrolling row of registry chips with its selection edge moving from the left border to
  a 2px bottom border; region padding drops 24 → 16px; the plate widens to 64% and drops to 26%
  opacity; and the two least load-bearing columns (Size, Last fetch) are removed from the table
  rather than being squeezed.

**Column dropping, not squeezing.** When width runs out, whole columns leave the table. State,
package and version never leave — they are the answer to the question the reader arrived with.

### Named Rules

**The No-Container Rule.** Regions are separated by full-width hairline rules, not by an inset
container with a background. A section's edge is a line across the whole sheet; the page never grows
a visible box around its content.

## Elevation & Depth

**This system has no elevation.** There are no ambient shadows, no blurs, no layered surfaces, and
no glow — the Monofolio `--cyber-glow` / `--steam-glow` utilities do not survive into this world.
Depth is entirely inked: a hairline changes weight, a value changes, or a texture appears. The two
tonal steps that exist (`--ground-raised`, `--ground-sunk`) are confirmation, not elevation, and
cannot be seen on their own (see The Undependable Fill Rule).

Two hard-edged offsets exist, and both are the bitmap world's own material rather than lighting —
zero blur, solid colour, on the primary action only:

### Shadow Vocabulary

- **Action ring** (`box-shadow: 0 0 0 2px var(--ground), 0 0 0 3px var(--accent)`): hover on the one
  primary action. A cut-out ring, not a halo.
- **Pixel step** (`box-shadow: 3px 0 0 var(--accent-ink), 6px 0 0 var(--accent)` with
  `transform: translateX(-2px)`): `:active` only. The button displaces sideways and leaves two
  stacked plates behind it, like a mis-registered print pull.

### The material layer

**The plate** is the system's one image: an authored raster (`halftone-plate.png`), a 45° halftone
screen at 60 lpi generated deterministically, running **99.7% ink coverage down to 6.3%** across its
width, emitted as 8-bit grayscale + alpha. It is applied as a CSS **mask** over a `--copper` fill, so
the ink stays a token rather than being baked into the file. It sits behind the specimen head at 34%
opacity (26% below 900px), `min(52%, 620px)` wide, anchored right, `pointer-events: none`,
`aria-hidden`.

Two rules the build encodes and Phase 2 must keep: the plate is **mirrored** (`scaleX(-1)`) so its
dense end bleeds off the outer margin instead of facing the caption — copper at 34% over ground
composites to roughly `#513021`, where `--ink-dim` would fall to 3.21:1; and its fill is
**transparent until masking is confirmed supported** (`@supports` gates `background: var(--copper)`),
because an engine or minifier that dropped the mask would otherwise paint a solid copper block into
exactly that failing condition.

**The dot field** is the second texture: a 9×9px radial dot grid in `--rule-soft` at 55% opacity,
gradient-masked to fade at both ends. It lives only in the masthead's flex spacer — the one box that
holds no text and cannot acquire any, and which collapses to zero width exactly when the bar wraps.
Measured behind wrapped 12px `--ink-dim`, the field drops that text to 4.50:1, so it is confined by
construction rather than tuned by opacity.

### Named Rules

**The Flat-At-Rest Rule.** Nothing casts a shadow at rest. The only two box-shadows in the system are
hard-edged, zero-blur, and belong to `:hover` and `:active` on the primary action. A soft or offset
shadow anywhere else — cards, panels, dropdowns, the masthead — is out of the world.

**The Texture-In-Empty-Boxes Rule.** A texture may only occupy a box that holds no text and cannot
come to hold text. Painting order is not contrast: if a field can end up behind a label, it is
confined, not faded.

## Shapes

**Zero radius, everywhere.** Every corner in the system is square — buttons, inputs, panels, the nav
frame, the state chips. Monofolio's incumbent 2px radius does not survive into this world; the square
corner is the bitmap face's own geometry, and rounding it re-introduces a different world's
softness.

**Everything is 1px.** Rules, control borders, region edges, the note's tie bar and the selected
registry's lit edge are all exactly 1px. The only 2px strokes in the system are the focus ring and
the mobile bay's selected underline. There is no 3px or thicker border, and no coloured side stripe
— a thick accent bar on a row is refused; the 1px lit edge plus ink does that job.

**Dashed means "at rest", solid means "engaged".** Row dividers are `1px dashed var(--rule-soft)`.
On hover the same divider becomes `1px solid var(--rule-strong)`. This is the replacement for the
retired decorative grid: the edge survives, but every line now separates two real things.

**The dot grid is the icon system.** The wordmark is a 3×3 grid of 4px squares on a 16px viewBox with
2px gutters, and the state matrices are the same figure at two resolutions. Icons are inline SVG at
14–16px with 1.5px strokes; there is no icon font and no glyph-as-icon.

## Components

Buttons, fields and rows are all built the same way: a 1px `--rule-strong` boundary, a square corner,
uppercase tracked mono, and no fill unless the control is the one action on the view.

### Buttons

- **Shape:** square (0 radius), 1px boundary.
- **Primary — the one filled action** (`.action`): crimson fill under counter-ink, Pixel Small
  uppercase, 12/16px padding. Exactly one per view. Hover paints the cut-out ring; `:active` fires
  the pixel step; disabled drops the fill entirely and becomes a dim outlined control (crimson never
  appears in a disabled state).
- **Secondary — the control** (`.ctl`): transparent, 1px `--rule-strong`, `--ink-dim` Meta uppercase,
  8/12px padding. Hover lifts the text to `--ink` and the border to `--ink-dim`. `aria-pressed="true"`
  lifts the text to `--ink` and turns the border copper. Disabled is `opacity: .5`.
- **Focus:** every control shows the shared amber ring (`2px solid var(--focus)`, 2px offset). Focus
  is never suppressed, never replaced by a colour change, and never inherited from the browser
  default.

### Inputs / Fields

- **Style:** a well, not a box — `--ground-sunk` behind a 1px `--rule-strong` border, 12px padding,
  Row-size text, a 14px inline SVG affordance in `--ink-dim`, and a visually-hidden label. The inner
  `<input>` is fully transparent and borderless; the wrapper is the control.
- **Focus:** `:focus-within` on the wrapper — border turns crimson *and* the amber ring is drawn on
  the wrapper. The ring is the accessible signal; the crimson border is the aesthetic one.
- **Placeholder:** `--ink-dim`, and it shows real example input (`serde, @scope/pkg`), never an
  instruction.

### Navigation

- **Primary nav** is a single 1px `--rule-strong` frame with hairline cell dividers and no gaps — one
  segmented block, not a row of links. Cells are Meta uppercase, `--ink-dim`, hover to `--ink`.
- **Current page reverses to a solid block**: `--ink` background, `--ground` text, weight 700. This is
  the system's one full contrast reversal, and it costs no scanning room.
- **The registry bay** is a ranked list, not a menu: Pixel Small registry name plus a right-aligned
  tabular count, 8/24px rows. The selected row takes `--ink` text, a crimson registry name, a 1px
  crimson left edge and the raised fill — four channels, because no single one of them is
  dependable. Below 900px it becomes a horizontal scroller and the left edge becomes a 2px bottom
  border.

### Cards / Containers

There are none. The system has no card. The only bounded box is the **panel** — a 1px
`--rule-strong` frame with 40/24px padding, used for empty, loading and error states, centred, with a
Pixel Small heading and body copy capped at 64ch. Its error variant turns the frame and the heading
crimson and nothing else. A panel never nests inside another panel and never carries a shadow.

### Data table

- **Head:** Meta uppercase at 0.14em tracking, `--ink-dim`, above a 1px solid `--rule-strong` rule.
  A visible `<caption>`, left-aligned, states the row count and the sort.
- **Rows:** 12px vertical padding, baseline-aligned, divided by 1px dashed `--rule-soft`.
- **Hover:** the raised fill, the divider goes solid `--rule-strong`, and the package name underlines
  in `--rule-strong`. Three channels again; the fill alone would be invisible.
- **Columns:** State and Version are fixed-width so the eye can track them down the page; the package
  name is the only fluid column. Numeric columns are right-aligned and tabular.

### Resolution as State (signature)

The system's memorable device, and the reason the world exists.

- **Fine** — a 3×3 matrix of 5px cells, 1px gutters: BatleHub holds this artifact and has verified it.
- **Coarse** — a 2×2 matrix of 8px cells, 1px gutters: it does not.
- Cells inherit `currentColor`; unlit cells sit at 18% opacity, lit cells at 100%. The matrix is
  `aria-hidden` and always accompanied by its word in Meta uppercase.
- The six states and their three channels:

| State | Matrix | Lit | Colour | Meaning |
|---|---|---|---|---|
| Cached | fine 3×3 | all 9 | `--ink` | held and verified |
| Stale | fine 3×3 | 8 of 9, centre out | `--copper` | held, past its freshness window |
| Held | coarse 2×2 | 3 of 4 | `--copper` | withheld by the release-age gate |
| Pending | coarse 2×2 | 2 of 4 | `--ink-dim` | not yet resolved |
| Yanked | coarse 2×2 | 1 of 4 | `--ink-dim` | withdrawn upstream |
| Blocked | coarse 2×2 | all 4 | `--accent` | refused by policy |

- **Motion.** One authored animation, `resolve`: `filter: blur(2.5px) contrast(1.7); opacity: .55` →
  none, over **520ms `cubic-bezier(.16, 1, .3, 1)`**. It plays on rows that have just arrived or just
  changed state — never on load of unchanged content, never on hover, never on more than the rows
  that changed. `prefers-reduced-motion: reduce` removes it and every transition in the system
  outright, with no crossfade substitute.

### The denial note

Every refusal states its rule in its own row, directly under the artifact it refused, tied to it with
`aria-describedby` — never one banner floating above the table. The note is Body-size `--ink-dim`,
introduced by a 1px full-height `--rule-strong` tie bar, with the rule's name in crimson and a single
inline link to the check that would explain it. A denial that cannot name its rule does not get a
note; it gets a state.

### Named Rules

**The Three Channel Rule.** Every state is carried by pattern, word and hue together. Any one of them
must be sufficient: a reader glancing from a second screen reads the matrix, a colour-blind reader
reads the word, a reader who knows the system reads the hue.

**The One Motion Rule.** The system animates exactly one thing — an artifact resolving. Nothing else
moves: no hover lifts, no page transitions, no skeleton shimmer, no easing on the fill changes.

**The Rule-Beside-Its-Row Rule.** Policy explanations live in the row of the thing they refused.
Never aggregate refusals into a summary banner.

## Do's and Don'ts

### Do:

- **Do** author every colour token in-gamut for sRGB as written, and keep the Monofolio source value
  as a provenance comment (The In-Gamut Rule).
- **Do** put dark `--accent-ink` on every crimson fill (5.69:1), never light ink (The Counter-Ink Rule).
- **Do** carry state on rule weight, ink and pattern, and treat `--ground-raised` / `--ground-sunk` as
  confirmation only — they are 1.06:1 and 1.005:1 (The Undependable Fill Rule).
- **Do** size Silkscreen at integer multiples of 8px and step it discretely per breakpoint
  (16 / 24 / 56 / 72 / 88 / 104).
- **Do** keep all small text at `--ink-dim` (5.62:1) or better, and never below 12px.
- **Do** give every state three channels — pattern, word, hue — and label every matrix with its word.
- **Do** drop whole table columns when width runs out; State, Package and Version stay at every size.
- **Do** confine texture to boxes that hold no text and cannot acquire any.
- **Do** keep the amber focus ring on everything focusable, at `2px` with `2px` offset.
- **Do** state each refusal's rule in its own row, tied by `aria-describedby`.
- **Do** move focus deliberately after any re-render that destroys the focused element — flip
  attributes in place where possible, and otherwise land focus on the surface the action produced.

### Don't:

- **Don't** introduce a card, a tile, or a stat row. The world refuses the card grid and the
  stat-tile row by name; regions are separated by full-width hairlines.
- **Don't** add a corner radius. Every corner is square, including ported Monofolio components that
  arrive with 2px.
- **Don't** add a shadow, a blur, or a glow. The only two box-shadows are the zero-blur ring and pixel
  step on the primary action; `--cyber-glow` and `--steam-glow` do not exist in this world.
- **Don't** paint a thick coloured side stripe to mark selection — 1px lit edge plus ink does it.
- **Don't** use `clamp()` or any fluid sizing on the bitmap face.
- **Don't** let hue alone carry a state, and don't add a hue to signal a new condition — the dot
  pattern carries it.
- **Don't** spend crimson outside its four jobs (link text, the one filled action, `blocked`,
  selection edge), and don't let it appear in a disabled control.
- **Don't** use copper to mean "healthy" — it means waiting or held.
- **Don't** ship a font as "the closest installed face"; both faces are self-hosted, preloaded, and
  set to `swap`.
- **Don't** use an icon font or a glyph-as-icon; icons are inline SVG at 14–16px, 1.5px stroke.
- **Don't** animate anything other than an artifact resolving, and don't substitute a crossfade when
  `prefers-reduced-motion` is set — remove the motion.
- **Don't** re-introduce the decorative two-axis grid background. Every line separates two real
  things.
