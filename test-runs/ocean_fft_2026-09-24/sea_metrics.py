#!/usr/bin/env python3
"""Tier-2 sea appearance numbers for the FFT calibration.

For each capture: fine contrast (std of luminance minus a 6px Gaussian blur,
x1000) and foam fraction (share of pixels with luminance > 0.72 and
saturation < 0.25) in named water regions, plus the lattice ring ratio.

Regions (1280x720): grid = overhead lattice crop; deck_near = deck foreground
(rows 470-720); deck_mid = the band just under the horizon (rows 360-440).
Usage: sea_metrics.py grid|deck capture.png [...]
"""
import sys

import numpy as np
from PIL import Image
from scipy.ndimage import gaussian_filter

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from lattice_metric import metric as lattice

REGIONS = {
    "grid": {"grid": (320, 40, 1280, 680)},
    "deck": {"deck_near": (0, 470, 1280, 720), "deck_mid": (0, 360, 1280, 440)},
}


def stats(rgb, box):
    x0, y0, x1, y1 = box
    region = rgb[y0:y1, x0:x1]
    lum = region @ np.array([0.2126, 0.7152, 0.0722])
    fine = lum - gaussian_filter(lum, 6.0)
    top = region.max(axis=2)
    saturation = (top - region.min(axis=2)) / np.maximum(top, 1e-6)
    foam = ((lum > 0.72) & (saturation < 0.25)).mean()
    return fine.std() * 1000.0, foam * 100.0


if __name__ == "__main__":
    kind = sys.argv[1]
    for path in sys.argv[2:]:
        rgb = np.asarray(Image.open(path).convert("RGB"), dtype=np.float64) / 255.0
        parts = []
        for name, box in REGIONS[kind].items():
            fine, foam = stats(rgb, box)
            parts.append(f"{name}: fine={fine:.1f} foam={foam:.2f}%")
        if kind == "grid":
            peak, _ = lattice(path)
            parts.append(f"lattice={peak:.1f}")
        print(f"{path.rsplit('/', 3)[-3]}/{path.rsplit('/', 1)[-1]}  " + "  ".join(parts))
