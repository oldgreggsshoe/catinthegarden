#!/usr/bin/env python3
"""Lattice metric for ocean captures.

Usage: lattice_metric.py IMG [IMG...] [--crop=x0,y0,x1,y1 as fractions]

Reports, for a centred 512x512 window of luma (8-bit grey levels):
  peak_levels  rms amplitude of the strongest spectral peak pair, in grey
               levels. The primary figure: a visible lattice is several levels.
  top8_share   % of all detail power held by the 4 strongest peak pairs.
  detail       rms of all detail, in grey levels.
  whitened     strongest peak / median after dividing by the radial mean
               power. Kept for comparison with older notes, but misleading
               once high frequencies are filtered: removing broadband sparkle
               lowers the median, so a sub-level residual peak scores high.
Calibration (25 Sept): Gerstner overhead lattice 3.7 levels / 3.6%; FFT sea
0.65-0.9 levels / 0.7-1.2%.
"""
import sys
import numpy as np
from PIL import Image


def score(path, crop=(0.0, 0.0, 1.0, 1.0), size=512):
    im = np.asarray(Image.open(path).convert("RGB"), dtype=np.float64)
    h, w, _ = im.shape
    x0, y0, x1, y1 = crop
    im = im[int(y0 * h):int(y1 * h), int(x0 * w):int(x1 * w)]
    g = im @ np.array([0.2126, 0.7152, 0.0722])
    n = min(min(g.shape), size)
    cy, cx = g.shape[0] // 2, g.shape[1] // 2
    g = g[cy - n // 2:cy + n // 2, cx - n // 2:cx + n // 2]
    g = g - g.mean()
    win = np.outer(np.hanning(n), np.hanning(n))
    p = np.abs(np.fft.fftshift(np.fft.fft2(g * win))) ** 2
    yy, xx = np.indices(p.shape)
    r = np.hypot(yy - n // 2, xx - n // 2).astype(int)
    band = (r >= n // 32) & (r <= n // 2 - 1)
    v = np.sort(p[band])[::-1]
    norm = (win ** 2).sum() * n * n
    peak_levels = np.sqrt(2 * v[:2].sum() / norm)
    top8_share = 100 * v[:8].sum() / v.sum()
    detail = np.sqrt(v.sum() / norm)
    radial = np.bincount(r.ravel(), p.ravel()) / np.maximum(np.bincount(r.ravel()), 1)
    white = (p / np.maximum(radial[r], 1e-30))[band]
    return peak_levels, top8_share, detail, white.max() / np.median(white)


if __name__ == "__main__":
    crop = (0.0, 0.0, 1.0, 1.0)
    for a in sys.argv[1:]:
        if a.startswith("--crop="):
            crop = tuple(float(t) for t in a[7:].split(","))
    for path in (a for a in sys.argv[1:] if not a.startswith("--")):
        peak, share, detail, white = score(path, crop)
        print(f"{path}: peak_levels={peak:.2f} top8_share={share:.2f}% "
              f"detail={detail:.1f} whitened={white:.1f}")
