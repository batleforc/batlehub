import { describe, it, expect } from "vitest";
import {
  variantFromMap,
  REGISTRY_TYPE_VARIANTS,
  sourceVariant,
  firewallVariant,
  severityVariant,
  eventBadgeVariant,
} from "./badge-variants";

describe("variantFromMap", () => {
  it("returns the mapped variant", () => {
    expect(variantFromMap("npm", REGISTRY_TYPE_VARIANTS)).toBe("default");
  });

  it("falls back when unmapped", () => {
    expect(variantFromMap("unknown", REGISTRY_TYPE_VARIANTS)).toBe("outline");
    expect(variantFromMap("unknown", REGISTRY_TYPE_VARIANTS, "secondary")).toBe("secondary");
  });
});

describe("sourceVariant", () => {
  it("maps known sources", () => {
    expect(sourceVariant("local")).toBe("secondary");
    expect(sourceVariant("both")).toBe("default");
    expect(sourceVariant("upstream")).toBe("outline");
  });
});

describe("firewallVariant", () => {
  it("maps status strings", () => {
    expect(firewallVariant(undefined)).toBe("outline");
    expect(firewallVariant("blocked")).toBe("destructive");
    expect(firewallVariant("yanked")).toBe("secondary");
    expect(firewallVariant("clear")).toBe("outline");
  });
});

describe("severityVariant", () => {
  it("maps severities", () => {
    expect(severityVariant("critical")).toBe("destructive");
    expect(severityVariant("high")).toBe("destructive");
    expect(severityVariant("medium")).toBe("default");
    expect(severityVariant("low")).toBe("secondary");
  });
});

describe("eventBadgeVariant", () => {
  it("maps notification event types", () => {
    expect(eventBadgeVariant("package_published")).toBe("default");
    expect(eventBadgeVariant("package_yanked")).toBe("destructive");
    expect(eventBadgeVariant("package_unyanked")).toBe("secondary");
    expect(eventBadgeVariant("other")).toBe("outline");
  });
});
