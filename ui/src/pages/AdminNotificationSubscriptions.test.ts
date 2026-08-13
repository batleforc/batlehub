import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";

const mocks = vi.hoisted(() => ({
  listSubscriptions: vi.fn(),
  listNotificationChannels: vi.fn(),
  createSubscription: vi.fn(),
  updateSubscription: vi.fn(),
  deleteSubscription: vi.fn(),
  testSubscription: vi.fn(),
  listRegistries: vi.fn(),
  explorePackages: vi.fn(),
  explorePackageDetail: vi.fn(),
  listSubjects: vi.fn(),
}));
vi.mock("@/client/sdk.gen", () => mocks);

import AdminNotificationSubscriptions from "./AdminNotificationSubscriptions.vue";

const sub = (over: Record<string, unknown> = {}) => ({
  id: "sub-1",
  registry: "npm",
  package_name: null,
  event_types: ["package_published"],
  channel_name: "my-slack",
  enabled: true,
  ...over,
});

async function mountPage() {
  const wrapper = mount(AdminNotificationSubscriptions, {
    // `attachTo` because the edit form lives in a `Dialog`, which teleports.
    attachTo: document.body,
    global: { stubs: { SectionTabs: true, RouterLink: true } },
  });
  await flushPromises();
  return wrapper;
}

/** The dialog is teleported out of the wrapper, so read the document. */
const dialogText = () => document.body.textContent ?? "";
const dialogButtons = () => [...document.body.querySelectorAll("button")];

const editButton = (w: Awaited<ReturnType<typeof mountPage>>) =>
  w.findAll("button").find((b) => /^edit$/i.test(b.text().trim()))!;

/** The page's question: "what gets notified where, and does it work". */
describe("AdminNotificationSubscriptions", () => {
  beforeEach(() => {
    mocks.listSubscriptions.mockReset().mockResolvedValue({ data: [sub()] });
    mocks.listNotificationChannels
      .mockReset()
      .mockResolvedValue({ data: [{ name: "my-slack", kind: "slack" }] });
    mocks.createSubscription.mockReset().mockResolvedValue({ data: {} });
    mocks.updateSubscription.mockReset().mockResolvedValue({ data: {} });
    mocks.deleteSubscription.mockReset().mockResolvedValue({ data: {} });
    mocks.testSubscription.mockReset().mockResolvedValue({ data: {} });
    mocks.listRegistries.mockReset().mockResolvedValue({ data: [{ name: "npm", type: "npm" }] });
    mocks.explorePackages.mockReset().mockResolvedValue({ data: { items: [], total: 0 } });
    mocks.listSubjects.mockReset().mockResolvedValue({ data: { items: [], truncated: false } });
  });

  /**
   * §4.3: an event type outside `ALL_EVENT_TYPES` is not silently re-saved.
   *
   * `openEdit` copies `sub.event_types` verbatim and the chip row renders only
   * the types this console knows, so a type the *server* knows and the console
   * does not was invisible in the form and re-saved on submit — an operator
   * kept a rule they could neither see nor remove. A server that grows an event
   * type before the console does is the expected order of deployment.
   */
  // `attachTo: document.body` leaves the teleported dialog behind, and the
  // assertions read the document — so a previous test's DOM would answer them.
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("names an event type it does not know rather than carrying it invisibly", async () => {
    mocks.listSubscriptions.mockResolvedValue({
      data: [sub({ event_types: ["package_published", "package_quarantined"] })],
    });
    const wrapper = await mountPage();
    await editButton(wrapper).trigger("click");
    await flushPromises();

    expect(dialogText()).toContain("package_quarantined");
    expect(dialogText()).toMatch(/does not know/i);
  });

  it("lets an unknown event type be removed", async () => {
    mocks.listSubscriptions.mockResolvedValue({
      data: [sub({ event_types: ["package_published", "package_quarantined"] })],
    });
    const wrapper = await mountPage();
    await editButton(wrapper).trigger("click");
    await flushPromises();

    const drop = dialogButtons().find((b) => b.textContent?.includes("package_quarantined"))!;
    drop.click();
    await flushPromises();

    expect(dialogText()).not.toContain("package_quarantined");
  });

  it("says nothing about unknown types when there are none", async () => {
    const wrapper = await mountPage();
    await editButton(wrapper).trigger("click");
    await flushPromises();
    expect(dialogText()).not.toMatch(/does not know/i);
  });

  /**
   * The colour of the test result used to be decided by
   * `testMsg.startsWith("Test failed")` — a translated sentence parsed for its
   * own meaning, correct in English and silently green in French.
   */
  it("reports a failed test as a failure, whatever language it is in", async () => {
    mocks.testSubscription.mockResolvedValue({ error: { message: "channel unreachable" } });
    const wrapper = await mountPage();
    await wrapper
      .findAll("button")
      .find((b) => /^test$/i.test(b.text().trim()))!
      .trigger("click");
    await flushPromises();

    expect(wrapper.text()).toContain("channel unreachable");
    expect((wrapper.vm as unknown as { testFailed: boolean }).testFailed).toBe(true);
  });

  it("surfaces a load error rather than an empty list", async () => {
    mocks.listSubscriptions.mockResolvedValue({ error: { message: "db down" } });
    const wrapper = await mountPage();
    expect(wrapper.text()).toContain("db down");
  });
});
