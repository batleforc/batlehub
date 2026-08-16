import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";

import Skeleton from "./Skeleton.vue";

describe("Skeleton", () => {
  it("renders the requested number of lines", () => {
    expect(
      mount(Skeleton, { props: { lines: 5 } }).findAll('[data-testid="skeleton-line"]'),
    ).toHaveLength(5);
  });

  /** A skeleton has nothing to say; the surface announces the real result. */
  it("is hidden from assistive tech", () => {
    expect(mount(Skeleton).attributes("aria-hidden")).toBe("true");
  });

  /**
   * `motion-safe:` rather than a bare animation — the craft floor and WCAG both
   * require honouring prefers-reduced-motion, and a pulse on a region the user
   * is already waiting for is exactly the kind that provokes it.
   */
  it("only animates when motion is safe", () => {
    const bar = mount(Skeleton).find('[data-testid="skeleton-line"]');
    expect(bar.classes()).toContain("motion-safe:animate-pulse");
    expect(bar.classes()).not.toContain("animate-pulse");
  });

  /** Ragged widths read as text; identical bars read as a loading graphic. */
  it("varies line widths and shortens the last one", () => {
    const bars = mount(Skeleton, { props: { lines: 4 } }).findAll('[data-testid="skeleton-line"]');
    const widths = bars.map((b) => b.attributes("style"));
    expect(new Set(widths).size).toBeGreaterThan(1);
    expect(widths.at(-1)).toContain("62%");
  });
});
