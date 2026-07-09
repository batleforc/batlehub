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

describe("forgejo", () => {
  const t = typeDef("forgejo");

  it("buildPaths requires owner and repo", () => {
    expect(t.buildPaths("forgejo", { owner: "", repo: "x" })).toEqual([]);
  });

  it("buildPaths gates ref/filename/filePath-dependent paths", () => {
    const paths = t.buildPaths("forgejo", { owner: "org", repo: "proj", ref: "", filename: "", filePath: "" });
    expect(paths.find((p) => p.label === "List releases")!.available).toBe(true);
    expect(paths.find((p) => p.label === "Release by tag")!.available).toBe(false);
    expect(paths.find((p) => p.label === "Asset by filename")!.available).toBe(false);
    expect(paths.find((p) => p.label === "Raw file")!.available).toBe(false);
    expect(paths.find((p) => p.label === "Package API passthrough")!.available).toBe(true);
  });

  it("buildPaths fills in all paths when every field is set", () => {
    const paths = t.buildPaths("forgejo", {
      owner: "org",
      repo: "proj",
      ref: "v1.0.0",
      filename: "app.tar.gz",
      filePath: "README.md",
    });
    expect(paths.every((p) => p.available)).toBe(true);
    expect(paths.find((p) => p.label === "Raw file")!.url).toBe(
      "/proxy/forgejo/org/proj/raw/v1.0.0/README.md",
    );
  });
});

describe("gitlab", () => {
  const t = typeDef("gitlab");

  it("buildPaths returns nothing without a project path", () => {
    expect(t.buildPaths("gitlab", { project: "" })).toEqual([]);
  });

  it("buildPaths gates tag/link/file-dependent paths", () => {
    const paths = t.buildPaths("gitlab", { project: "group/proj", tag: "", linkName: "", filePath: "" });
    expect(paths.find((p) => p.label === "List releases")!.available).toBe(true);
    expect(paths.find((p) => p.label === "Release by tag")!.available).toBe(false);
    expect(paths.find((p) => p.label === "Release link download")!.available).toBe(false);
    expect(paths.find((p) => p.label === "Raw file")!.available).toBe(false);
    expect(paths.find((p) => p.label === "API v4 passthrough")!.available).toBe(true);
  });

  it("buildPaths fills in all paths when every field is set", () => {
    const paths = t.buildPaths("gitlab", {
      project: "group/proj",
      tag: "v1.0.0",
      linkName: "app.bin",
      filePath: "README.md",
    });
    expect(paths.every((p) => p.available)).toBe(true);
    expect(paths.find((p) => p.label === "Release link download")!.url).toBe(
      "/proxy/gitlab/group/proj/-/releases/v1.0.0/downloads/app.bin",
    );
  });

  it("parses a gitlab.com tag URL", () => {
    const parser = t.urlParsers!.find((p) => p.matchesHost("gitlab.com"))!;
    expect(parser.parse(["group", "proj", "-", "releases", "tags", "v1.0.0"])).toEqual({
      project: "group/proj",
      tag: "v1.0.0",
    });
  });

  it("parses a gitlab.com archive URL", () => {
    const parser = t.urlParsers!.find((p) => p.matchesHost("gitlab.com"))!;
    expect(parser.parse(["group", "proj", "-", "archive", "v2.0.0"])).toEqual({
      project: "group/proj",
      tag: "v2.0.0",
    });
  });

  it("parses a gitlab.com releases URL without /tags/", () => {
    const parser = t.urlParsers!.find((p) => p.matchesHost("gitlab.com"))!;
    expect(parser.parse(["group", "proj", "-", "releases", "v3.0.0"])).toEqual({
      project: "group/proj",
      tag: "v3.0.0",
    });
  });

  it("returns empty when the URL has no '-' separator", () => {
    const parser = t.urlParsers!.find((p) => p.matchesHost("gitlab.com"))!;
    expect(parser.parse(["group", "proj"])).toEqual({});
  });
});

describe("nuget", () => {
  const t = typeDef("nuget");

  it("buildPaths gates id/version-dependent paths", () => {
    const paths = t.buildPaths("nuget", { package: "", version: "" });
    expect(paths.find((p) => p.label === "Service index")!.available).toBe(true);
    expect(paths.find((p) => p.label === "Flat — versions list")!.available).toBe(false);
    expect(paths.find((p) => p.label === "Flat — .nupkg download")!.available).toBe(false);
  });

  it("buildPaths fills in all paths with package + version", () => {
    const paths = t.buildPaths("nuget", { package: "Newtonsoft.Json", version: "13.0.3" });
    expect(paths.every((p) => p.available)).toBe(true);
    expect(paths.find((p) => p.label === "Flat — .nupkg download")!.url).toBe(
      "/proxy/nuget/nuget/v3/flat/Newtonsoft.Json/13.0.3/Newtonsoft.Json.13.0.3.nupkg",
    );
  });

  it("parses a nuget.org package URL", () => {
    const parser = t.urlParsers!.find((p) => p.matchesHost("nuget.org"))!;
    expect(parser.parse(["packages", "Newtonsoft.Json", "13.0.3"])).toEqual({
      package: "Newtonsoft.Json",
      version: "13.0.3",
    });
    expect(parser.parse(["not-packages"])).toEqual({});
  });
});

describe("goproxy", () => {
  const t = typeDef("goproxy");

  it("returns nothing without a module", () => {
    expect(t.buildPaths("goproxy", { module: "", version: "" })).toEqual([]);
  });

  it("gates version-dependent paths", () => {
    const paths = t.buildPaths("goproxy", { module: "github.com/gin-gonic/gin", version: "" });
    expect(paths.find((p) => p.label === "Latest version")!.available).toBe(true);
    expect(paths.find((p) => p.label === "go.mod file")!.available).toBe(false);
  });

  it("fills in version-dependent paths", () => {
    const paths = t.buildPaths("goproxy", {
      module: "github.com/gin-gonic/gin",
      version: "v1.9.1",
    });
    expect(paths.find((p) => p.label === "Module zip (.zip)")!.url).toBe(
      "/proxy/goproxy/github.com/gin-gonic/gin@v/v1.9.1.zip",
    );
  });
});

describe("terraform", () => {
  const t = typeDef("terraform");

  it("gates module and provider paths independently", () => {
    const paths = t.buildPaths("terraform", {
      namespace: "hashicorp",
      name: "",
      provider: "aws",
      version: "",
      os: "",
      arch: "",
    });
    expect(paths.find((p) => p.label === "Module versions")!.available).toBe(false);
    expect(paths.find((p) => p.label === "Provider versions")!.available).toBe(true);
    expect(paths.find((p) => p.label === "Provider download")!.available).toBe(false);
  });

  it("fills in module and provider paths when fully specified", () => {
    const paths = t.buildPaths("terraform", {
      namespace: "hashicorp",
      name: "consul",
      provider: "aws",
      version: "1.0.0",
      os: "linux",
      arch: "amd64",
    });
    expect(paths.every((p) => p.available)).toBe(true);
    expect(paths.find((p) => p.label === "Module download")!.url).toBe(
      "/proxy/terraform/v1/modules/hashicorp/consul/aws/1.0.0/download",
    );
    expect(paths.find((p) => p.label === "Provider artifact")!.url).toBe(
      "/proxy/terraform/v1/providers/hashicorp/aws/1.0.0/artifact/linux/amd64",
    );
  });
});

describe("openvsx", () => {
  const t = typeDef("openvsx");

  it("requires publisher, name, and version for the VSIX download", () => {
    expect(t.buildPaths("openvsx", { publisher: "", name: "python", version: "1.0" })[0].available).toBe(
      false,
    );
    const paths = t.buildPaths("openvsx", {
      publisher: "ms-python",
      name: "python",
      version: "2024.0.0",
    });
    expect(paths[0]).toEqual({
      label: "VSIX download",
      url: "/proxy/openvsx/ms-python.python/2024.0.0/vsix",
      available: true,
    });
  });
});

describe("jetbrains", () => {
  const t = typeDef("jetbrains");

  it("uses a placeholder when no path is given", () => {
    const paths = t.buildPaths("jetbrains", { path: "" });
    expect(paths[0]).toEqual({
      label: "File download",
      url: "/proxy/jetbrains/jetbrains/<product>/<filename>",
      available: false,
    });
  });

  it("uses the given path when set", () => {
    const paths = t.buildPaths("jetbrains", { path: "idea/ideaIC-2024.1.4.tar.gz" });
    expect(paths[0]).toEqual({
      label: "File download",
      url: "/proxy/jetbrains/jetbrains/idea/ideaIC-2024.1.4.tar.gz",
      available: true,
    });
  });
});

describe("conda buildPaths", () => {
  const t = typeDef("conda");

  it("defaults platform to linux-64 when blank and gates the package file", () => {
    const paths = t.buildPaths("conda", { platform: "", filename: "" });
    expect(paths[0].url).toBe("/proxy/conda/linux-64/repodata.json");
    expect(paths.find((p) => p.label === "Package file")!.available).toBe(false);
  });

  it("uses a custom platform and includes the package file when set", () => {
    const paths = t.buildPaths("conda", { platform: "osx-64", filename: "numpy-1.26.0.conda" });
    expect(paths.find((p) => p.label === "Package file")).toEqual({
      label: "Package file",
      url: "/proxy/conda/osx-64/numpy-1.26.0.conda",
      available: true,
    });
  });
});

describe("deb", () => {
  const t = typeDef("deb");

  it("uses a placeholder path when blank", () => {
    const paths = t.buildPaths("deb", { path: "" });
    expect(paths[0].available).toBe(false);
    expect(paths[0].url).toContain("<suite>/<component>/<file>");
    expect(paths[1]).toEqual({
      label: "Signing key",
      url: "/proxy/deb/deb/key.gpg",
      available: true,
    });
  });

  it("uses the given path", () => {
    const paths = t.buildPaths("deb", { path: "stable/main/Packages.gz" });
    expect(paths[0]).toEqual({
      label: "Repository path",
      url: "/proxy/deb/deb/stable/main/Packages.gz",
      available: true,
    });
  });
});

describe("rpm", () => {
  const t = typeDef("rpm");

  it("uses a placeholder path when blank", () => {
    const paths = t.buildPaths("rpm", { path: "" });
    expect(paths[0].available).toBe(false);
    expect(paths[1].available).toBe(true);
  });

  it("uses the given path", () => {
    const paths = t.buildPaths("rpm", { path: "repodata/repomd.xml" });
    expect(paths[0]).toEqual({
      label: "Repository path",
      url: "/proxy/rpm/rpm/repodata/repomd.xml",
      available: true,
    });
  });
});

describe("pacman", () => {
  const t = typeDef("pacman");

  it("defaults arch to x86_64 and gates the package file", () => {
    const paths = t.buildPaths("pacman", { arch: "", filename: "" });
    expect(paths.find((p) => p.label === "Package file")!.available).toBe(false);
    expect(paths.find((p) => p.label === "Repository DB")!.url).toBe(
      "/proxy/pacman/pacman/x86_64/pacman.db",
    );
  });

  it("uses a custom arch and includes the package file when set", () => {
    const paths = t.buildPaths("pacman", { arch: "aarch64", filename: "hello-1.0-1.pkg.tar.zst" });
    expect(paths[0]).toEqual({
      label: "Package file",
      url: "/proxy/pacman/pacman/aarch64/hello-1.0-1.pkg.tar.zst",
      available: true,
    });
  });
});

describe("cargo", () => {
  const t = typeDef("cargo");

  it("returns nothing without a crate name", () => {
    expect(t.buildPaths("cargo", { name: "", version: "" })).toEqual([]);
  });

  it("gates version-dependent paths", () => {
    const paths = t.buildPaths("cargo", { name: "serde", version: "" });
    expect(paths.find((p) => p.label === "Version metadata")!.available).toBe(false);
    expect(paths.find((p) => p.label === "Sparse index config")!.available).toBe(true);
  });

  it("fills in version-dependent paths", () => {
    const paths = t.buildPaths("cargo", { name: "serde", version: "1.0.197" });
    expect(paths.find((p) => p.label === ".crate download")!.url).toBe(
      "/proxy/cargo/serde/1.0.197/download",
    );
  });

  it("parses a crates.io URL with and without a version", () => {
    const parser = t.urlParsers!.find((p) => p.matchesHost("crates.io"))!;
    expect(parser.parse(["crates", "serde", "1.0.197"])).toEqual({
      name: "serde",
      version: "1.0.197",
    });
    expect(parser.parse(["crates", "serde", "download"])).toEqual({ name: "serde", version: "" });
    expect(parser.parse(["nope"])).toEqual({});
  });
});

describe("rubygems", () => {
  const t = typeDef("rubygems");

  it("gates name-dependent paths and always exposes the spec indexes", () => {
    const paths = t.buildPaths("rubygems", { name: "", version: "" });
    expect(paths.find((p) => p.label === "Full specs index")!.available).toBe(true);
    expect(paths.find((p) => p.label === "Gem info (JSON)")!.available).toBe(false);
    expect(paths.find((p) => p.label === "Gem download")!.available).toBe(false);
  });

  it("fills in name/version-dependent paths", () => {
    const paths = t.buildPaths("rubygems", { name: "rails", version: "7.1.3" });
    expect(paths.find((p) => p.label === "Gem download")!.url).toBe(
      "/proxy/rubygems/gems/rails-7.1.3.gem",
    );
  });

  it("parses a rubygems.org gem URL, with and without a version", () => {
    const parser = t.urlParsers!.find((p) => p.matchesHost("rubygems.org"))!;
    expect(parser.parse(["gems", "rails"])).toEqual({ name: "rails" });
    expect(parser.parse(["gems", "rails", "versions", "7.1.3"])).toEqual({
      name: "rails",
      version: "7.1.3",
    });
    expect(parser.parse(["nope"])).toEqual({});
  });
});

describe("composer", () => {
  const t = typeDef("composer");

  it("gates name/version-dependent paths", () => {
    const paths = t.buildPaths("composer", { vendor: "", package: "", version: "" });
    expect(paths.find((p) => p.label === "Root index")!.available).toBe(true);
    expect(paths.find((p) => p.label === "Package metadata (p2)")!.available).toBe(false);
    expect(paths.find((p) => p.label === "Dist download")!.available).toBe(false);
  });

  it("fills in vendor/package/version-dependent paths", () => {
    const paths = t.buildPaths("composer", { vendor: "symfony", package: "console", version: "7.1.0" });
    expect(paths.find((p) => p.label === "Dist download")!.url).toBe(
      "/proxy/composer/dist/symfony/console/7.1.0",
    );
  });

  it("parses a packagist p2 URL", () => {
    const parser = t.urlParsers!.find((p) => p.matchesHost("packagist.org"))!;
    expect(parser.parse(["p2", "symfony", "console.json"])).toEqual({
      vendor: "symfony",
      package: "console",
    });
    expect(parser.parse(["p2", "symfony", "console~dev.json"])).toEqual({
      vendor: "symfony",
      package: "console",
    });
  });

  it("parses a packagist packages URL", () => {
    const parser = t.urlParsers!.find((p) => p.matchesHost("packagist.org"))!;
    expect(parser.parse(["packages", "symfony", "console"])).toEqual({
      vendor: "symfony",
      package: "console",
    });
    expect(parser.parse(["nope"])).toEqual({});
  });
});
