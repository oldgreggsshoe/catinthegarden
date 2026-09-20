#!/usr/bin/env python3
"""Ground saturation and tonal spread, for a capture set and for the references.

The one metric round 13 turns on: `(max-min)/max` per pixel over the lower 65%
of the frame, which is ground rather than sky, plus the p99-p01 luminance
spread. Run it against any capture directory to compare with the photographs.
"""
import sys, glob
import numpy as np
from PIL import Image

LUMA = [0.2126, 0.7152, 0.0722]


def frame_stats(path):
    a = np.asarray(Image.open(path).convert("RGB")).astype(float) / 255
    high, low = a.max(axis=2), a.min(axis=2)
    saturation = np.where(high > 1e-6, (high - low) / np.maximum(high, 1e-6), 0)
    luminance = a @ LUMA
    ground = slice(int(a.shape[0] * 0.35), None)
    return (
        saturation[ground].mean(),
        np.percentile(luminance[ground], 99) - np.percentile(luminance[ground], 1),
    )


def main(paths):
    rows = [(p, *frame_stats(p)) for p in paths]
    for path, saturation, spread in rows:
        print(f"{path.split('/')[-1][:44]:46s} sat {saturation:.3f}  spread {spread:.3f}")
    print(
        f"{'MEAN':46s} sat {np.mean([r[1] for r in rows]):.3f}"
        f"  spread {np.mean([r[2] for r in rows]):.3f}"
    )


if __name__ == "__main__":
    args = sys.argv[1:]
    files = []
    for arg in args:
        files.extend(sorted(glob.glob(arg)))
    if not files:
        print("usage: measure.py 'run/screenshots/capture-*.png'")
        raise SystemExit(1)
    main(files)
