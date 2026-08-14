---
version: 1
slug: "website-vitepress-theme-custom-css"
primary_target: "website/.vitepress/theme/custom.css"
related_targets: ["website/index.md","website/guide/installation.md"]
---

Scope: the public documentation site (`website/`, becoming `docs/` under RFC 0005)
— 41 published pages across guide, registries and, after the merge, operations,
contributing and rfc. Visitor mode: Read, including the home page. A docs index is
Read, not Persuade: the visitor is evaluating a self-hosted product, and hype
costs credibility with the audience PRODUCT.md names.

Audience and job: the same three audiences as the console, met earlier. The
self-hoster deciding whether to install at all; the platform owner looking up a
config key; the developer who wants one snippet for their package manager. Unlike
the console, the dominant visit is *reading for minutes*, not scanning for
seconds — the surface has to stay comfortable, not just legible.

Chosen direction: the console's world, applied. Not a new visual world — DESIGN.md
is authoritative and this surface adopts it rather than proposing anything. What
the site wears today (`--bh-*` tokens, glows, 2px radii, IBM Plex Sans, Google
Fonts) is the world DESIGN.md replaced and is being removed, not extended.

READING ROLE — DECIDED, and this is the record of the pick. RFC 0005 open question
1 asked for the prose step, line height and measure, because DESIGN.md's ramp was
authored for a console and has no long-form reading role. Three candidates were
built against real content from `guide/installation.md` and measured in a browser
at 1440, dark rendition:

  A  15px / 1.6  / 72ch  → rendered 71 chars, 9.0px leading   (console Row step)
  B  16px / 1.7  / 68ch  → rendered 67 chars, 11.2px leading  ← CHOSEN
  C  16px / 1.75 / 62ch  → rendered 61 chars, 12.0px leading

B is chosen. The step is 16px because that is the ordinary web body floor and
because DESIGN.md already declares Sub (16px) and explicitly parks it — "declared
in the ramp but not exercised by this surface… treat their usage as unset, not as
established". This is the surface it was waiting for; no new token is invented.
Line height goes to 1.7 rather than the console's 1.6 for three reasons that
compound: a 67-character line is long, light-on-dark needs compensation, and a
monospace face gives no word-shape cue for the return sweep. Measure sits
mid-band; C's 61ch reads well per line but breaks technical identifiers
(`ghcr.io/batleforc/batlehub:<version>`) noticeably more often, which is a real
cost in this content.

JetBrains Mono's advance is exactly 0.6em — measured, 9.0px at 15px and 9.6px at
16px — so `ch` is the true character count here and the 45–75 band applies
literally rather than approximately.

Tracking is NOT part of the compensation. The generic guidance for light-on-dark
asks for leading, tracking and weight; DESIGN.md's Tracking Ladder Rule ends with
"Lowercase text is never tracked", and the system wins. The leading carries it
alone. Weight stays 400: ink on ground measures 16.88:1, and weight compensation
is for low-contrast situations this palette does not have.

Constraints: WCAG 2.2 AA; the ramp does not change between renditions (DESIGN.md);
Silkscreen only ever at integer multiples of 8px (The Integer Em Rule); no glow,
no shadow at rest, zero radius; fonts self-hosted — the site's current Google
Fonts import is the same defect that left the console's text face unpainted under
its own CSP.

HEADING RANK — DECIDED, and worth recording because the measurement looks like a
defect and is not one. A Silkscreen 16px h2 renders a 27.2px box while a
JetBrains Mono 20px h3 under it renders 34px: the subordinate heading is
physically larger. Accepted. The rank here is not carried by size — the face
changes, the case changes and the tracking changes, and those read as a level
before size is consulted. DESIGN.md's own note is the reason this works rather
than a problem to solve: Pixel Small "is the size at which the bitmap face is
still a label and not yet an image", and a label reads as a heading precisely
because it is set in the other face.

Two conditions would break it, and phase 3 must check them rather than assume:
an h3 long enough to wrap to two lines, and the 390px rendition, where the
reflowed column narrows the gap the face change is carrying.

Unresolved: nothing on this surface has been verified against the real VitePress
build, because RFC 0005 phases 1–2 have not landed — the measurements above come
from a standalone proof using the real tokens and the real faces, at 1440 in the
dark rendition only.
