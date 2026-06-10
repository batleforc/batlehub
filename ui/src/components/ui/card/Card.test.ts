import { describe, it, expect } from "vitest";
import { mount } from "@vue/test-utils";
import Card from "./Card.vue";
import CardHeader from "./CardHeader.vue";
import CardTitle from "./CardTitle.vue";
import CardDescription from "./CardDescription.vue";
import CardContent from "./CardContent.vue";
import CardFooter from "./CardFooter.vue";

describe("Card", () => {
  it("renders slot content in a div with base classes", () => {
    const wrapper = mount(Card, { slots: { default: "Body" } });
    expect(wrapper.text()).toBe("Body");
    expect(wrapper.element.tagName).toBe("DIV");
    expect(wrapper.classes()).toContain("rounded-sm");
    expect(wrapper.classes()).toContain("border");
  });

  it("merges a custom class", () => {
    const wrapper = mount(Card, { props: { class: "my-card" }, slots: { default: "Body" } });
    expect(wrapper.classes()).toContain("my-card");
    expect(wrapper.classes()).toContain("bg-card");
  });
});

describe("CardHeader", () => {
  it("renders slot content in a div with base classes", () => {
    const wrapper = mount(CardHeader, { slots: { default: "Header" } });
    expect(wrapper.text()).toBe("Header");
    expect(wrapper.element.tagName).toBe("DIV");
    expect(wrapper.classes()).toContain("flex");
    expect(wrapper.classes()).toContain("flex-col");
  });

  it("merges a custom class", () => {
    const wrapper = mount(CardHeader, { props: { class: "my-header" }, slots: { default: "x" } });
    expect(wrapper.classes()).toContain("my-header");
  });
});

describe("CardTitle", () => {
  it("renders slot content in an h3 with base classes", () => {
    const wrapper = mount(CardTitle, { slots: { default: "Title" } });
    expect(wrapper.text()).toBe("Title");
    expect(wrapper.element.tagName).toBe("H3");
    expect(wrapper.classes()).toContain("font-mono");
    expect(wrapper.classes()).toContain("font-bold");
  });

  it("merges a custom class", () => {
    const wrapper = mount(CardTitle, { props: { class: "my-title" }, slots: { default: "x" } });
    expect(wrapper.classes()).toContain("my-title");
  });
});

describe("CardDescription", () => {
  it("renders slot content in a p with base classes", () => {
    const wrapper = mount(CardDescription, { slots: { default: "Description" } });
    expect(wrapper.text()).toBe("Description");
    expect(wrapper.element.tagName).toBe("P");
    expect(wrapper.classes()).toContain("text-muted-foreground");
  });

  it("merges a custom class", () => {
    const wrapper = mount(CardDescription, {
      props: { class: "my-desc" },
      slots: { default: "x" },
    });
    expect(wrapper.classes()).toContain("my-desc");
  });
});

describe("CardContent", () => {
  it("renders slot content in a div with base classes", () => {
    const wrapper = mount(CardContent, { slots: { default: "Content" } });
    expect(wrapper.text()).toBe("Content");
    expect(wrapper.element.tagName).toBe("DIV");
    expect(wrapper.classes()).toContain("p-6");
    expect(wrapper.classes()).toContain("pt-0");
  });

  it("merges a custom class", () => {
    const wrapper = mount(CardContent, {
      props: { class: "my-content" },
      slots: { default: "x" },
    });
    expect(wrapper.classes()).toContain("my-content");
  });
});

describe("CardFooter", () => {
  it("renders slot content in a div with base classes", () => {
    const wrapper = mount(CardFooter, { slots: { default: "Footer" } });
    expect(wrapper.text()).toBe("Footer");
    expect(wrapper.element.tagName).toBe("DIV");
    expect(wrapper.classes()).toContain("flex");
    expect(wrapper.classes()).toContain("items-center");
  });

  it("merges a custom class", () => {
    const wrapper = mount(CardFooter, {
      props: { class: "my-footer" },
      slots: { default: "x" },
    });
    expect(wrapper.classes()).toContain("my-footer");
  });
});
