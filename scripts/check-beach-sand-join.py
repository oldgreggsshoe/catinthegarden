#!/usr/bin/env python3
"""Check the reproduced vertical sand seam: python script.py RUN_DIRECTORY.

Requires numpy and Pillow. This ROI belongs to the fixed beach_sand_join pose,
not arbitrary screenshots. Fails on the original dry-land colour discontinuity.
"""

import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image


def check(root):
    manifest = json.loads((root / "manifest.json").read_text())
    assert manifest["scenario"] == "beach_sand_join" and manifest["passed"]
    paths = sorted((root / "screenshots").glob("capture-*.png"))
    assert len(paths) == 3, "requires all three captures"
    results = []
    for path in paths:
        pixels = np.asarray(Image.open(path).convert("RGB"), dtype=float)
        assert pixels.shape == (720, 1280, 3), "use the replay's 1280x720 viewport"
        # The join crosses x=659/660. Include texture variation either side
        # instead of choosing an artificially smooth pair of individual pixels.
        jumps = np.abs(np.diff(pixels[300:700, 652:668], axis=1)).mean(axis=2)
        strongest_per_row = jumps.max(axis=1)
        p95 = float(np.percentile(strongest_per_row, 95))
        results.append({"capture": path.name, "p95_rgb_step": p95})
    print(json.dumps(results, indent=2))
    assert all(row["p95_rgb_step"] <= 3.0 for row in results), "hard dry-land/beach colour join"


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    check(Path(sys.argv[1]))
