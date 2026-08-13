import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";

const { authFetchMock } = vi.hoisted(() => ({ authFetchMock: vi.fn() }));
vi.mock("@/composables/useAuthFetch", () => ({
  useAuthFetch: () => ({ authFetch: authFetchMock }),
}));

import AdminConfigReload from "./AdminConfigReload.vue";

const soon = () => new Date(Date.now() + 5 * 60_000).toISOString();
const past = () => new Date(Date.now() - 60_000).toISOString();

const pending = (over: Record<string, unknown> = {}) => ({
  id: "11111111-1111-1111-1111-111111111111",
  created_at: new Date().toISOString(),
  expires_at: soon(),
  source: "file_watcher",
  diff: {
    added_registries: [],
    removed_registries: [],
    changed_registries: [],
    access_config_changed: false,
    limits_changed: false,
  },
  warnings: [],
  ...over,
});

/** Route every endpoint this page calls; `pending` is the one under test. */
function routes(pendingBody: unknown | null) {
  authFetchMock.mockImplementation((url: string) => {
    const json = url.includes("/config/pending")
      ? pendingBody
      : url.includes("/config/warnings")
        ? { warnings: [] }
        : url.includes("/config/content")
          ? { content: "", readonly: false, hot_reload_enabled: true }
          : url.includes("/config/changes")
            ? { items: [], total: 0, page: 0, per_page: 50 }
            : {};
    return Promise.resolve({
      ok: pendingBody !== null || !url.includes("/config/pending"),
      status: pendingBody === null && url.includes("/config/pending") ? 404 : 200,
      json: async () => json,
    });
  });
}

async function mountPage() {
  const wrapper = mount(AdminConfigReload, {
    global: { stubs: { SectionTabs: true, RouterLink: true } },
  });
  await flushPromises();
  await flushPromises();
  return wrapper;
}

const applyButton = (w: Awaited<ReturnType<typeof mountPage>>) =>
  w.findAll("button").find((b) => b.text().trim() === "Apply");

/**
 * The page's question: "what is about to change, and do I accept it".
 *
 * §4.3's two assertions: a diff carrying only `access_config_changed` still
 * renders a decision surface, and an expired pending disables Apply.
 */
describe("AdminConfigReload", () => {
  beforeEach(() => {
    authFetchMock.mockReset();
  });

  /**
   * A change to RBAC alone used to render *nothing at all* — an empty badge row
   * that invites a blind apply, on the operator's decision surface.
   */
  it("renders a decision surface for an access-config-only change", async () => {
    routes(pending({ diff: { ...pending().diff, access_config_changed: true } }));
    const wrapper = await mountPage();

    expect(wrapper.text()).toMatch(/access control changed/i);
    expect(applyButton(wrapper)).toBeDefined();
  });

  it("disables Apply once the pending reload has expired", async () => {
    routes(pending({ expires_at: past() }));
    const wrapper = await mountPage();

    expect(applyButton(wrapper)?.attributes("disabled")).toBeDefined();
  });

  it("leaves Apply available while the pending reload is live", async () => {
    routes(pending());
    const wrapper = await mountPage();

    expect(applyButton(wrapper)?.attributes("disabled")).toBeUndefined();
  });

  /**
   * A4: the snapshot carries the *candidate's* warnings.
   *
   * `PendingReload` has held them since it was written and the snapshot dropped
   * them, so the one surface that exists to review a change before applying it
   * could not show what the change would warn about. The page worked around it
   * by remembering the `validate` call — which is lost on any page reload, and
   * never existed for a file-watcher reload nobody staged by hand.
   */
  it("shows what the candidate config would warn about", async () => {
    routes(
      pending({
        warnings: [
          {
            code: "registry_no_upstreams",
            message: "Registry 'legacy' has no upstreams configured.",
            path: "registries.legacy.upstreams",
          },
        ],
      }),
    );
    const wrapper = await mountPage();

    expect(wrapper.text()).toContain("registries.legacy.upstreams");
    expect(wrapper.text()).toContain("no upstreams configured");
  });

  it("says there is nothing staged rather than rendering an empty decision", async () => {
    routes(null);
    const wrapper = await mountPage();
    expect(wrapper.text()).toMatch(/no pending reload/i);
    expect(applyButton(wrapper)).toBeUndefined();
  });
});
