import { describe, it, expect, vi, afterEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";

import AppFooter from "./AppFooter.vue";
import { REPORT_BUG_URL, REPORT_SECURITY_URL } from "@/config";

function stubFetch(impl: () => Promise<Response>) {
  const fetchMock = vi.fn(impl);
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

const json = (body: unknown, status = 200) =>
  Promise.resolve(new Response(JSON.stringify(body), { status }));

async function mountFooter() {
  const wrapper = mount(AppFooter);
  await flushPromises();
  return wrapper;
}

/**
 * The footer reports which build is running — an operator reading a bug report
 * needs it — and it must do so *best-effort*: the version comes from a live
 * `/healthz` call, and a server that is down or upgrading must not take the
 * page's links with it.
 */
describe("AppFooter", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("shows the running version once /healthz answers", async () => {
    stubFetch(() => json({ version: "1.2.3" }));
    const wrapper = await mountFooter();
    expect(wrapper.text()).toContain("1.2.3");
  });

  /*
   * `not.toContain("Version")` is what these three asserted first, and it can
   * never fail: `appFooter.version` is "BatleHub v{version}" in both
   * catalogues, so the word "Version" is not on the page in any state. The
   * assertion has to name the rendered prefix — rendering the line
   * unconditionally as "BatleHub vunknown" left all three green.
   */
  const VERSION_PREFIX = "BatleHub v";

  it("omits the version rather than showing a broken line when /healthz fails", async () => {
    stubFetch(() => json({}, 503));
    const wrapper = await mountFooter();
    expect(wrapper.text()).not.toContain(VERSION_PREFIX);
    expect(wrapper.findAll("a")).toHaveLength(2);
  });

  it("omits the version when /healthz has no version field", async () => {
    stubFetch(() => json({ status: "ok" }));
    const wrapper = await mountFooter();
    expect(wrapper.text()).not.toContain(VERSION_PREFIX);
  });

  it("survives an unreachable server", async () => {
    stubFetch(() => Promise.reject(new Error("network down")));
    const wrapper = await mountFooter();
    expect(wrapper.element.tagName).toBe("FOOTER");
    expect(wrapper.text()).not.toContain(VERSION_PREFIX);
  });

  it("points the report links off-site, safely", async () => {
    stubFetch(() => json({ version: "1.2.3" }));
    const wrapper = await mountFooter();
    const links = wrapper.findAll("a");
    expect(links.map((a) => a.attributes("href"))).toEqual([REPORT_BUG_URL, REPORT_SECURITY_URL]);
    for (const link of links) {
      expect(link.attributes("target")).toBe("_blank");
      expect(link.attributes("rel")).toBe("noopener noreferrer");
    }
  });
});
