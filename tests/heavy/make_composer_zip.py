#!/usr/bin/env python3
"""Build a minimal Composer package zip, for tests/heavy/composer.sh.

    make_composer_zip.py <out.zip> <vendor/package> <version> [require-name require-constraint]

`parse_composer_zip` (crates/adapters/src/registry/composer/local.rs) reads
`composer.json` from the archive root or from a single top-level directory, and
takes the version from the file when the upload does not override it. Composer
itself needs the autoload target to exist, or `composer require` installs a
package whose classes cannot be loaded — which would pass a "did it install"
assertion and fail the first `use` statement.
"""

import json
import sys
import zipfile


def main() -> int:
    out, name, version = sys.argv[1:4]
    require = {}
    if len(sys.argv) > 5:
        require[sys.argv[4]] = sys.argv[5]

    vendor, package = name.split("/", 1)
    root = f"{vendor}-{package}-{version}"
    class_name = "Probe"

    composer_json = {
        "name": name,
        "version": version,
        "description": "RFC 0009 heavy test probe",
        "type": "library",
        "license": "MIT",
        "require": require or {"php": ">=7.4"},
        "autoload": {"psr-4": {"HeavyProbe\\": "src/"}},
    }

    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr(f"{root}/composer.json", json.dumps(composer_json, indent=2))
        zf.writestr(
            f"{root}/src/{class_name}.php",
            f"<?php\nnamespace HeavyProbe;\nclass {class_name} "
            f"{{ const NAME = '{name}'; }}\n",
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
