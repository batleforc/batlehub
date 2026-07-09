import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import PackageEventsTable from "./PackageEventsTable.vue";
import type { PackageEventDto } from "@/client/types.gen";

function event(over: Partial<PackageEventDto> = {}): PackageEventDto {
  return {
    id: "1",
    timestamp: "2026-01-01T00:00:00Z",
    user_id: "alice",
    user_role: "admin",
    version: "1.0.0",
    artifact: "pkg-1.0.0.tgz",
    action: "download",
    outcome: "allowed",
    deny_reason: null,
    ...over,
  } as PackageEventDto;
}

describe("PackageEventsTable", () => {
  it("shows the empty state when there are no events", () => {
    const wrapper = mount(PackageEventsTable, { props: { events: [] } });
    expect(wrapper.text()).toContain("No events recorded yet.");
  });

  it("renders a row per event with mapped action label", () => {
    const wrapper = mount(PackageEventsTable, { props: { events: [event()] } });
    expect(wrapper.findAll("tbody tr")).toHaveLength(1);
    expect(wrapper.text()).toContain("Download");
    expect(wrapper.text()).toContain("alice");
    expect(wrapper.text()).toContain("pkg-1.0.0.tgz");
  });

  it("falls back to the raw action string for an unknown action", () => {
    const wrapper = mount(PackageEventsTable, {
      props: { events: [event({ action: "publish" })] },
    });
    expect(wrapper.text()).toContain("publish");
  });

  it("shows 'anonymous' when user_id is absent", () => {
    const wrapper = mount(PackageEventsTable, {
      props: { events: [event({ user_id: null })] },
    });
    expect(wrapper.text()).toContain("anonymous");
  });

  it("shows '—' when artifact and deny_reason are absent", () => {
    const wrapper = mount(PackageEventsTable, {
      props: { events: [event({ artifact: null, deny_reason: null })] },
    });
    const cells = wrapper.findAll("td");
    expect(cells.some((c) => c.text() === "—")).toBe(true);
  });

  it("uses the destructive badge variant for a denied outcome", () => {
    const wrapper = mount(PackageEventsTable, {
      props: { events: [event({ outcome: "denied", deny_reason: "blocked" })] },
    });
    expect(wrapper.text()).toContain("denied");
    expect(wrapper.text()).toContain("blocked");
  });
});
