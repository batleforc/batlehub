import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import { Pagination } from ".";

describe("Pagination", () => {
  it("displays a 1-indexed page label for a 0-indexed page prop", () => {
    const wrapper = mount(Pagination, { props: { page: 0 } });
    expect(wrapper.text()).toContain("Page 1");
  });

  it("shows the total when totalPages is provided", () => {
    const wrapper = mount(Pagination, { props: { page: 2, totalPages: 5 } });
    expect(wrapper.text()).toContain("Page 3 of 5");
  });

  it("disables Previous on the first page", () => {
    const wrapper = mount(Pagination, { props: { page: 0 } });
    const buttons = wrapper.findAll("button");
    expect(buttons[0].attributes("disabled")).toBeDefined();
  });

  it("disables Next on the last page when totalPages is known", () => {
    const wrapper = mount(Pagination, { props: { page: 4, totalPages: 5 } });
    const buttons = wrapper.findAll("button");
    expect(buttons[1].attributes("disabled")).toBeDefined();
  });

  it("disables Next when hasNext is false and totalPages is unknown", () => {
    const wrapper = mount(Pagination, { props: { page: 0, hasNext: false } });
    const buttons = wrapper.findAll("button");
    expect(buttons[1].attributes("disabled")).toBeDefined();
  });

  it("emits update:page with the previous/next page index", async () => {
    const wrapper = mount(Pagination, { props: { page: 1, totalPages: 5 } });
    const buttons = wrapper.findAll("button");
    await buttons[0].trigger("click");
    await buttons[1].trigger("click");
    expect(wrapper.emitted("update:page")?.[0]).toEqual([0]);
    expect(wrapper.emitted("update:page")?.[1]).toEqual([2]);
  });

  it("disables both buttons when disabled prop is set", () => {
    const wrapper = mount(Pagination, { props: { page: 1, totalPages: 5, disabled: true } });
    const buttons = wrapper.findAll("button");
    expect(buttons[0].attributes("disabled")).toBeDefined();
    expect(buttons[1].attributes("disabled")).toBeDefined();
  });
});
