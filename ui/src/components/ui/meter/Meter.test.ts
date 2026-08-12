import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import { Meter } from ".";

/**
 * The meter ships with its accessibility contract tested (RFC 0004 §10),
 * because a bar with no name and no value text is a decoration that looks like
 * information. Every assertion here is one a screen-reader user depends on.
 */
const base = {
  value: 820,
  max: 1024,
  label: "Storage",
  valueText: "820 B of 1.0 KiB",
};

const meter = (props: Partial<typeof base> & Record<string, unknown> = {}) =>
  mount(Meter, { props: { ...base, ...props } });

describe("Meter", () => {
  it("exposes role=meter with the full value triple", () => {
    const el = meter().get('[role="meter"]');
    expect(el.attributes("aria-valuenow")).toBe("820");
    expect(el.attributes("aria-valuemin")).toBe("0");
    expect(el.attributes("aria-valuemax")).toBe("1024");
  });

  /**
   * `aria-valuenow: 820` is not the fact. "820 B of 1.0 KiB" is, and it has to
   * be announced, not merely painted.
   */
  it("announces the same sentence a sighted reader sees", () => {
    const wrapper = meter();
    expect(wrapper.get('[role="meter"]').attributes("aria-valuetext")).toBe("820 B of 1.0 KiB");
    expect(wrapper.text()).toContain("820 B of 1.0 KiB");
  });

  it("takes its accessible name from the visible label", () => {
    const wrapper = meter();
    const labelledBy = wrapper.get('[role="meter"]').attributes("aria-labelledby");
    expect(labelledBy).toBeTruthy();
    const label = wrapper.get(`#${labelledBy}`);
    expect(label.text()).toBe("Storage");
  });

  it("does not announce the fill a second time", () => {
    const fill = meter().get('[role="meter"] > div');
    expect(fill.attributes("aria-hidden")).toBe("true");
  });

  // ── the proportion ────────────────────────────────────────────────────────

  it("renders the fill in proportion to the limit", () => {
    const fill = meter({ value: 512, max: 1024 }).get('[role="meter"] > div');
    expect(fill.attributes("style")).toContain("width: 50%");
  });

  /**
   * `enforcement = "warn"` records a publish past the limit, so usage above
   * `max` is reachable. A bar wider than its track would overflow the layout
   * rather than say anything; the `at-limit` state is what says it.
   */
  it("clamps a value past the limit to a full bar", () => {
    const fill = meter({ value: 5_000, max: 1_000 }).get('[role="meter"] > div');
    expect(fill.attributes("style")).toContain("width: 100%");
  });

  it("renders empty rather than dividing by a zero limit", () => {
    const fill = meter({ value: 10, max: 0 }).get('[role="meter"] > div');
    expect(fill.attributes("style")).toContain("width: 0%");
  });

  it("clamps a negative value to an empty bar", () => {
    const fill = meter({ value: -5, max: 100 }).get('[role="meter"] > div');
    expect(fill.attributes("style")).toContain("width: 0%");
  });

  // ── state ─────────────────────────────────────────────────────────────────

  it("colours the fill per state, and exposes the state as data", () => {
    for (const [state, cls] of [
      ["ok", "bg-foreground"],
      ["warning", "bg-copper"],
      ["at-limit", "bg-destructive"],
    ] as const) {
      const wrapper = meter({ state });
      expect(wrapper.get('[role="meter"]').attributes("data-state")).toBe(state);
      expect(wrapper.get('[role="meter"] > div').classes()).toContain(cls);
    }
  });

  /**
   * DESIGN.md: state is never carried by hue alone. Strip every colour class
   * and the meter must still say what it says — which it does, because the
   * numbers are written out and announced.
   */
  it("still reads with no colour at all", () => {
    const wrapper = meter({ state: "at-limit", valueText: "1.0 KiB of 1.0 KiB" });
    expect(wrapper.text()).toContain("1.0 KiB of 1.0 KiB");
    expect(wrapper.get('[role="meter"]').attributes("aria-valuetext")).toBe("1.0 KiB of 1.0 KiB");
  });

  it("defaults to the ordinary state", () => {
    expect(meter().get('[role="meter"]').attributes("data-state")).toBe("ok");
  });

  /**
   * Two meters on one page is the normal case — one per quota-gated registry —
   * and a shared id would point both `aria-labelledby` references at the same
   * label, so one of them would announce the wrong registry's name.
   */
  it("gives each meter on a page its own label id", () => {
    const page = mount(
      {
        components: { Meter },
        template: `
          <div>
            <Meter :value="1" :max="10" label="npm" value-text="1 of 10" />
            <Meter :value="2" :max="10" label="cargo" value-text="2 of 10" />
          </div>`,
      },
      { global: { components: { Meter } } },
    );

    const ids = page.findAll('[role="meter"]').map((m) => m.attributes("aria-labelledby"));
    expect(ids).toHaveLength(2);
    expect(new Set(ids).size, `both meters used ${ids[0]}`).toBe(2);

    // …and each id resolves to its own label.
    expect(page.get(`#${ids[0]}`).text()).toBe("npm");
    expect(page.get(`#${ids[1]}`).text()).toBe("cargo");
  });
});
