#!/usr/bin/env python3
"""Lattice metric for ocean captures: strongest off-centre spectral peak vs median.

Usage: lattice_metric.py IMG [IMG...] [--crop x0,y0,x1,y1 as fractions]
Score = max(whitened power) / median(whitened power) in the mid-frequency
annulus. Whitening divides by the radial mean so the natural 1/f falloff does
not count as structure. Higher = more regular lattice; near-isotropic ~ low.
"""
import sys
import numpy as np
from PIL import Image

def score(path, crop=(0.0, 0.0, 1.0, 1.0), size=512):
    im = np.asarray(Image.open(path).convert("RGB"), dtype=np.float64) / 255.0
    h, w, _ = im.shape
    x0, y0, x1, y1 = crop
    im = im[int(y0 * h):int(y1 * h), int(x0 * w):int(x1 * w)]
    g = im @ np.array([0.2126, 0.7152, 0.0722])
    n = min(g.shape)
    n = min(n, size)
    cy, cx = g.shape[0] // 2, g.shape[1] // 2
    g = g[cy - n // 2:cy + n // 2, cx - n // 2:cx + n // 2]
    g = g - g.mean()
    win = np.outer(np.hanning(n), np.hanning(n))
    p = np.abs(np.fft.fftshift(np.fft.fft2(g * win))) ** 2
    yy, xx = np.indices(p.shape)
    r = np.hypot(yy - n // 2, xx - n // 2).astype(int)
    radial = np.bincount(r.ravel(), p.ravel()) / np.maximum(np.bincount(r.ravel()), 1)
    white = p / np.maximum(radial[r], 1e-30)
    band = (r >= n // 32) & (r <= n // 2 - 1)
    v = white[band]
    return float(v.max() / np.median(v)), float(np.percentile(v, 99.9) / np.median(v))

if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    crop = (0.0, 0.0, 1.0, 1.0)
    for a in sys.argv[1:]:
        if a.startswith("--crop="):
            crop = tuple(float(t) for t in a[7:].split(","))
    for p in args:
        m, q = score(p, crop)
        print(f"{p}: peak/median={m:.1f} p99.9/median={q:.1f}")
