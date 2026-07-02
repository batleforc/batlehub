import { describe, it, expect } from "vitest";
import { REGISTRY_PATH_TYPES } from "./registryPathFields";

function typeDef(id: string) {
  const t = REGISTRY_PATH_TYPES.find((x) => x.id === id);
  if (!t) throw new Error(`no such registry path type: ${id}`);
  return t;
}

describe("REGISTRY_PATH_TYPES", () => {
  it("has exactly the 18 expected registry types", () => {
    const ids = REGISTRY_PATH_TYPES.map((t) => t.id).sort();
    expect(ids).toEqual(
      [
        "cargo",
        "composer",
        "conda",
        "deb",
        "forgejo",
        "github",
        "gitlab",
        "goproxy",
        "jetbrains",
        "maven",
        "npm",
        "nuget",
        "openvsx",
        "pacman",
        "pypi",
        "rpm",
        "rubygems",
        "terraform",
      ].sort(),
    );
  });
});

describe("github", () => {
  const t = typeDef("github");

  it("buildPaths requires owner and repo", () => {
    expect(t.buildPaths("github", { owner: "", repo: "batlehub" })).toEqual([]);
  });

  it("buildPaths returns release/tarball/zip/asset paths gated on available fields", () => {
    const paths = t.buildPaths("github", {
      owner: "batleforc",
      repo: "ProxyAuthK8S",
      ref: "v0.1.9",
      assetId: "123",
      filename: "tool.tar.gz",
      filePath: "README.md",
    });
    expect(paths).toEqual([
      {
        label: "List releases",
        url: "/proxy/github/batleforc/ProxyAuthK8S/releases",
        available: true,
      },
      {
        label: "Release by tag",
        url: "/proxy/github/batleforc/ProxyAuthK8S/releases/tags/v0.1.9",
        available: true,
      },
      {
        label: "Source tarball",
        url: "/proxy/github/batleforc/ProxyAuthK8S/tarball/v0.1.9",
        available: true,
      },
      {
        label: "Zip archive",
        url: "/proxy/github/batleforc/ProxyAuthK8S/zipball/v0.1.9",
        available: true,
      },
      {
        label: "Asset by filename",
        url: "/proxy/github/batleforc/ProxyAuthK8S/releases/download/v0.1.9/tool.tar.gz",
        available: true,
      },
      {
        label: "Asset by ID",
        url: "/proxy/github/batleforc/ProxyAuthK8S/releases/assets/123",
        available: true,
      },
      {
        label: "Raw file",
        url: "/proxy/github/batleforc/ProxyAuthK8S/raw/v0.1.9/README.md",
        available: true,
      },
    ]);
  });

  it("parses a github.com/tag URL", () => {
    const parser = t.urlParsers!.find((p) => p.matchesHost("github.com"))!;
    expect(parser.parse(["owner", "repo", "releases", "tag", "v1.2.3"])).toEqual({
      owner: "owner",
      repo: "repo",
      ref: "v1.2.3",
    });
  });

  it("parses a github.com download URL with a filename", () => {
    const parser = t.urlParsers!.find((p) => p.matchesHost("github.com"))!;
    expect(
      parser.parse(["owner", "repo", "releases", "download", "v1.2.3", "asset.tar.gz"]),
    ).toEqual({ owner: "owner", repo: "repo", ref: "v1.2.3", filename: "asset.tar.gz" });
  });

  it("parses a raw.githubusercontent.com URL", () => {
    const parser = t.urlParsers!.find((p) => p.matchesHost("raw.githubusercontent.com"))!;
    expect(parser.parse(["owner", "repo", "main", "path", "to", "file.yaml"])).toEqual({
      owner: "owner",
      repo: "repo",
      ref: "main",
      filePath: "path/to/file.yaml",
    });
  });

  it("parses an api.github.com asset URL", () => {
    const parser = t.urlParsers!.find((p) => p.matchesHost("api.github.com"))!;
    expect(parser.parse(["repos", "owner", "repo", "releases", "assets", "999"])).toEqual({
      owner: "owner",
      repo: "repo",
      assetId: "999",
    });
  });
});

describe("npm", () => {
  const t = typeDef("npm");

  it("buildPaths omits version-gated paths without a version", () => {
    expect(t.buildPaths("npm", { package: "lodash", version: "" })).toEqual([
      { label: "Packument (all versions)", url: "/proxy/npm/lodash", available: true },
      { label: "Version metadata", url: "/proxy/npm/lodash/", available: false },
      { label: "Tarball download", url: "/proxy/npm/lodash//tarball", available: false },
    ]);
  });

  it("buildPaths includes version-gated paths with a version", () => {
    const paths = t.buildPaths("npm", { package: "lodash", version: "4.17.21" });
    expect(paths[1]).toEqual({
      label: "Version metadata",
      url: "/proxy/npm/lodash/4.17.21",
      available: true,
    });
  });

  it("returns an empty list without a package", () => {
    expect(t.buildPaths("npm", { package: "", version: "" })).toEqual([]);
  });

  it("parses a plain npmjs.org URL", () => {
    const parser = t.urlParsers![0];
    expect(parser.parse(["lodash", "4.17.21"])).toEqual({ package: "lodash", version: "4.17.21" });
  });

  it("parses a scoped-package tarball URL (- placeholder)", () => {
    const parser = t.urlParsers![0];
    expect(parser.parse(["lodash", "-", "lodash-4.17.21.tgz"])).toEqual({
      package: "lodash",
      version: "4.17.21",
    });
  });
});

describe("pypi", () => {
  const t = typeDef("pypi");

  it("has two independent url parsers for pypi.org and files.pythonhosted.org", () => {
    expect(
      t
        .urlParsers!.find((p) => p.matchesHost("pypi.org"))!
        .parse(["project", "requests", "2.31.0"]),
    ).toEqual({
      package: "requests",
      version: "2.31.0",
    });
    expect(
      t
        .urlParsers!.find((p) => p.matchesHost("files.pythonhosted.org"))!
        .parse(["packages", "ab", "cd", "requests-2.31.0-py3-none-any.whl"]),
    ).toEqual({ filename: "requests-2.31.0-py3-none-any.whl" });
  });
});

describe("maven", () => {
  const t = typeDef("maven");

  it("converts dotted group ids to slash paths", () => {
    const paths = t.buildPaths("maven", {
      group: "com.google.guava",
      artifact: "guava",
      version: "32.1.3-jre",
      filename: "",
    });
    expect(paths[0].url).toBe("/proxy/maven/maven2/com/google/guava/guava/");
    expect(paths[2].url).toBe(
      "/proxy/maven/maven2/com/google/guava/guava/32.1.3-jre/guava-32.1.3-jre.jar",
    );
  });
});

describe("conda and pacman defaults", () => {
  it("conda platform field defaults to linux-64", () => {
    const field = typeDef("conda").fields.find((f) => f.key === "platform")!;
    expect(field.default).toBe("linux-64");
  });

  it("pacman arch field defaults to x86_64", () => {
    const field = typeDef("pacman").fields.find((f) => f.key === "arch")!;
    expect(field.default).toBe("x86_64");
  });
});
