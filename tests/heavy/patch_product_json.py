#!/usr/bin/env python3
"""Point a VS Code build's extension gallery at a BatleHub instance.

Used by `tests/heavy/marketplace.sh` scenario 3b, which installs an extension
**by id** to prove BatleHub can serve as an editor's marketplace rather than
just cache its bytes.

The block written here is the one the console's Setup Guide publishes
(`ui/src/config/registryTypes.ts`), so this test fails if that snippet and the
routes ever disagree — which is the point. The key is `extensionsGallery`,
which is what VS Code and VSCodium actually read.

Usage: BASE=http://127.0.0.1:8080 patch_product_json.py <path to product.json>
"""

import json
import os
import sys


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2

    path = sys.argv[1]
    base = f"{os.environ['BASE']}/proxy/vscode"

    with open(path, encoding="utf-8") as fh:
        product = json.load(fh)

    product["extensionsGallery"] = {
        "serviceUrl": f"{base}/vscode/gallery",
        "itemUrl": f"{base}/vscode/item",
        "resourceUrlTemplate": (
            f"{base}/vscode/unpkg/{{publisher}}/{{name}}/{{version}}/{{path}}"
        ),
    }

    with open(path, "w", encoding="utf-8") as fh:
        json.dump(product, fh)

    print(f"extensionsGallery -> {base}/vscode/gallery")
    return 0


if __name__ == "__main__":
    sys.exit(main())
