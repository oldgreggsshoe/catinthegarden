#!/usr/bin/env python3
"""Lattice (regular-pattern) metric for an overhead ocean capture.

Takes the luminance of a water-only crop, removes its mean, windows it, and
reads the 2D power spectrum. For every ring of spatial frequency, it takes the
ratio of the strongest bin to the ring's mean. A few plane waves put all of a
ring's power into a handful of bins, so the ratio is large; a broadband sea
spreads it around the ring, so it stays near the ratio noise gives (~5-8).

Reports the largest ring ratio over rings 6..160 cycles/frame, and the median
over those rings. Usage: lattice_metric.py capture.png [more.png ...]
"""
import sys

import numpy as np
from PIL import Image


def metric(path, crop=(320, 40, 1280, 680)):
    image = np.asarray(Image.open(path).convert("RGB"), dtype=np.float64) / 255.0
    x0, y0, x1, y1 = crop
    lum = image[y0:y1, x0:x1] @ np.array([0.2126, 0.7152, 0.0722])
    lum = lum - lum.mean()
    h, w = lum.shape
    window = np.outer(np.hanning(h), np.hanning(w))
    power = np.abs(np.fft.fftshift(np.fft.fft2(lum * window))) ** 2
    yy, xx = np.indices(power.shape)
    # Frequency in cycles per frame, normalised to the shorter side so rings
    # are circles in cycles-per-pixel.
    fy = (yy - h // 2) / h
    fx = (xx - w // 2) / w
    radius = np.hypot(fx, fy) * min(h, w)
    ratios = []
    for r in range(6, 160):
        ring = power[(radius >= r) & (radius < r + 1)]
        if ring.size < 16:
            continue
        ratios.append(ring.max() / ring.mean())
    ratios = np.array(ratios)
    return ratios.max(), np.median(ratios)


if __name__ == "__main__":
    for path in sys.argv[1:]:
        peak, median = metric(path)
        print(f"{path}: peak_ring_ratio={peak:.1f} median_ring_ratio={median:.1f}")
