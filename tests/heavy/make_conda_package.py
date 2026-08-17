#!/usr/bin/env python3
"""Build a minimal conda `.tar.bz2` package, for tests/heavy/conda.sh.

    make_conda_package.py <out.tar.bz2> <name> <version> <build> <subdir> [depends...]

A conda package is a tarball whose `info/index.json` is the only thing a
registry has to understand — BatleHub's `parse_conda_metadata` reads exactly
that member (crates/adapters/src/registry/conda/client.rs). `info/paths.json`
and `info/files` are here because the *installer* reads them, and a package
that a registry accepts but micromamba refuses to link would prove half of what
this test is for.

Built here rather than with conda-build: the package under test is BatleHub's
handling of it, and a build tool would add a toolchain to CI that this proves
nothing about.
"""

import bz2
import io
import json
import sys
import tarfile


def add(tar: tarfile.TarFile, name: str, payload: bytes) -> None:
    info = tarfile.TarInfo(name)
    info.size = len(payload)
    # A fixed mtime: the package bytes must be identical between two runs that
    # publish the same content, or a checksum assertion becomes a clock reading.
    info.mtime = 0
    tar.addfile(info, io.BytesIO(payload))


def main() -> int:
    out, name, version, build, subdir = sys.argv[1:6]
    depends = list(sys.argv[6:])

    index = {
        "name": name,
        "version": version,
        "build": build,
        "build_number": 0,
        "subdir": subdir,
        "platform": subdir.split("-")[0] if "-" in subdir else None,
        "arch": subdir.split("-")[1] if "-" in subdir else None,
        "depends": depends,
        "license": "MIT",
        "timestamp": 0,
    }

    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w") as tar:
        add(tar, "info/index.json", json.dumps(index, indent=2).encode())
        # No payload files: the point is the metadata path, and an empty
        # package still has to link cleanly.
        add(tar, "info/paths.json", json.dumps({"paths": [], "paths_version": 1}).encode())
        add(tar, "info/files", b"")
        add(tar, "info/about.json", json.dumps({"summary": "RFC 0009 heavy probe"}).encode())

    with open(out, "wb") as fh:
        fh.write(bz2.compress(buf.getvalue()))
    return 0


if __name__ == "__main__":
    sys.exit(main())
