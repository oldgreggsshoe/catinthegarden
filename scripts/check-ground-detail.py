#!/usr/bin/env python3
"""Check underfoot texture: python script.py BEFORE_RUN AFTER_RUN.

Requires numpy and Pillow. ROIs belong only to ground_detail_village at Low,
1280x647 captured window size. This rejects the gain-only trial that darkened
smooth blobs without restoring grain; it does not replace visual/motion review.
"""

import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageFilter


def load(root):
    manifest = json.loads((root / "manifest.json").read_text())
    assert manifest["scenario"] == "ground_detail_village" and manifest["passed"]
    image = Image.open(root / "screenshots/capture-001.png").convert("RGB")
    pixels = np.asarray(image, dtype=float)
    assert pixels.shape == (647, 1280, 3), "use the documented Low 1280x647 capture"
    blurred = np.asarray(image.filter(ImageFilter.GaussianBlur(3)), dtype=float)
    high_pass = (pixels - blurred) @ np.array([0.2126, 0.7152, 0.0722])
    return pixels, float(high_pass[440:625, 80:980].std())


def check(before, after):
    a, old_grain = load(before)
    b, new_grain = load(after)
    assert old_grain > 0.0
    ratio = new_grain / old_grain
    sky_same = np.array_equal(a[10:90, 400:1000], b[10:90, 400:1000])
    inn_same = np.array_equal(a[30:190, 30:245], b[30:190, 30:245])
    print(json.dumps({"before_grain": old_grain, "after_grain": new_grain,
                      "contrast_ratio": ratio, "sky_identical": sky_same,
                      "inn_wall_identical": inn_same}, indent=2))
    assert sky_same and inn_same, "camera/lighting/geometry controls changed"
    assert ratio >= 1.35, "fine ground grain is still suppressed or absent"


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    check(Path(sys.argv[1]), Path(sys.argv[2]))
