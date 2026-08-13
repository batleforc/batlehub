import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";

const { auditLogMock } = vi.hoisted(() => ({ auditLogMock: vi.fn() }));
vi.mock("@/client/sdk.gen", () => ({ auditLog: auditLogMock }));

import AuditLog from "./AuditLog.vue";

const event = (over: Record<string, unknown> = {}) => ({
  id: crypto.randomUUID(),
  package_id: { registry: "npm", name: "left-pad", version: "1.0.0" },
  user_id: "alice",
  user_role: "user",
  action: "download",
  result: { outcome: "allowed" },
  timestamp: "2026-08-12T10:00:00Z",
  ...over,
});

/** What the endpoint actually answers with: the paginated envelope. */
const envelope = (items: unknown[], over: Record<string, unknown> = {}) => ({
  items,
  total: items.length,
  page: 0,
  per_page: 100,
  ...over,
});

async function mountPage() {
  const wrapper = mount(AuditLog, {
    global: { stubs: { RouterLink: { template: "<a><slot /></a>" }, SectionTabs: true } },
  });
  await flushPromises();
  return wrapper;
}

describe("AuditLog", () => {
  beforeEach(() => {
    auditLogMock.mockReset().mockResolvedValue({ data: envelope([event(), event({ user_id: "bob" })]) });
  });

  /**
   * The regression this file exists for. The page declared
   * `useApi<AccessEvent[]>` against a hand-written interface while the endpoint
   * returns `{ items, total, page, per_page }`, so it rendered "No events
   * recorded yet." over a full page of events — verified in a browser against
   * six fixture rows before the fix.
   *
   * On an audit surface that is the worst failure mode available: it does not
   * look broken, it looks like nothing happened.
   */
  it("renders the events inside the response envelope", async () => {
    const wrapper = await mountPage();
    expect(wrapper.findAll("tbody tr")).toHaveLength(2);
    expect(wrapper.text()).toContain("alice");
    expect(wrapper.text()).toContain("bob");
    expect(wrapper.text()).not.toContain("No events recorded yet");
  });

  it("says nothing happened only when nothing happened", async () => {
    auditLogMock.mockResolvedValue({ data: envelope([]) });
    const wrapper = await mountPage();
    expect(wrapper.findAll("tbody tr")).toHaveLength(0);
  });

  /**
   * `total` is the server's count; `items.length` is the page. Reporting the
   * page as the total silently caps at `per_page`.
   */
  it("reports the server's total, not the loaded page's length", async () => {
    auditLogMock.mockResolvedValue({ data: envelope([event()], { total: 4_312 }) });
    const wrapper = await mountPage();
    expect(wrapper.text()).toContain("4312");
  });

  it("filters by user against the envelope's items", async () => {
    const wrapper = await mountPage();
    const input = wrapper.findAll("input")[0];
    await input.setValue("bob");
    await flushPromises();
    expect(wrapper.findAll("tbody tr")).toHaveLength(1);
    expect(wrapper.text()).toContain("bob");
  });

  /**
   * `package_id` is null for account- and network-wide actions — blocking a
   * user, blocking an IP, purging the trail. The hand-written interface
   * declared it required, so the template dereferenced it unguarded and those
   * rows would have thrown once the envelope fix let them render at all. The
   * generated type is what made this visible.
   */
  it("renders an account-wide action, which carries no package", async () => {
    auditLogMock.mockResolvedValue({
      data: envelope([
        event({ package_id: null, action: "block_user", user_id: "admin" }),
        event(),
      ]),
    });
    const wrapper = await mountPage();
    expect(wrapper.findAll("tbody tr")).toHaveLength(2);
    expect(wrapper.text()).toContain("block_user");
    expect(wrapper.text()).toMatch(/account-wide/i);
  });

  it("surfaces an error rather than an empty log", async () => {
    auditLogMock.mockResolvedValue({ error: { message: "boom" } });
    const wrapper = await mountPage();
    expect(wrapper.text()).toContain("boom");
  });
});
