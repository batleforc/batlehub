import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";

import UpstreamNotice from "./UpstreamNotice.vue";

function notice(
  upstream: Partial<{
    attempted: boolean;
    freshness: string | null;
    version_count: number;
    truncated: boolean;
    error: string | null;
  }>,
  upstreamVersionCount = 0,
) {
  return mount(UpstreamNotice, {
    props: {
      upstream: {
        attempted: true,
        freshness: "fresh",
        version_count: upstreamVersionCount,
        truncated: false,
        error: null,
        ...upstream,
      },
      upstreamVersionCount,
    },
  });
}

describe("UpstreamNotice", () => {
  /**
   * Nothing happened, so there is nothing to say. A banner reading "we did not
   * ask upstream" on every internal package would be noise, and noise is what
   * teaches readers to stop reading banners.
   */
  it("says nothing when the read was not attempted", () => {
    expect(notice({ attempted: false }).text()).toBe("");
  });

  /**
   * Likewise when the upstream knew nothing this instance did not already hold:
   * the table is complete either way.
   */
  it("says nothing when upstream added no rows", () => {
    expect(notice({ attempted: true }, 0).text()).toBe("");
  });

  it("counts the rows upstream added", () => {
    expect(notice({}, 3).text()).toContain("3");
  });

  /**
   * A silently shortened list is a lie about the registry, so truncation gets
   * its own sentence rather than the same one with a different number.
   */
  it("says when the list was shortened", () => {
    const text = notice({ truncated: true }, 300).text();
    expect(text).toContain("300");
    expect(text.toLowerCase()).toContain("more");
  });

  /**
   * Rung 3 with a stale document: the page says how it is short rather than
   * presenting an old answer as a current one.
   */
  it("says when the answer came from a stale document", () => {
    expect(notice({ freshness: "stale" }, 2).text().toLowerCase()).toContain(
      "older cached",
    );
  });

  /**
   * Rung 3 with nothing to fall back to. The error wins over everything else:
   * a reader who is told "2 versions exist upstream" while the upstream is
   * unreachable has been told two contradictory things.
   */
  it("reports an unreachable upstream, and that reading wins", () => {
    const wrapper = notice(
      { error: "connection refused", freshness: "stale" },
      2,
    );
    expect(wrapper.text().toLowerCase()).toContain("could not be reached");
    expect(wrapper.text()).not.toContain("2 version");
  });
});
