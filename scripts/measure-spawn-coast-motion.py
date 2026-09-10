#!/usr/bin/env python3
"""Compare wavedir_spawn captures: python script.py DISABLED_RUN ENABLED_RUN.

Requires numpy and Pillow. Measures rendered pattern displacement, not fluid
velocity. Positive onshore cosine means motion toward the surveyed spawn shore.
"""

import json
import sys
from pathlib import Path

import numpy as np
from PIL import Image


def measure(root):
    manifest = json.loads((root / "manifest.json").read_text())
    assert manifest["scenario"] == "wavedir_spawn" and manifest["passed"]
    scenario = json.loads(
        (Path(__file__).resolve().parents[1] / "crates/app/scenarios/wavedir_spawn.json").read_text()
    )
    pose = scenario["waypoints"][0]
    forward = np.array(pose["look_at"]) - np.array(pose["position"])
    forward /= np.linalg.norm(forward)
    right = np.cross(forward, [0, 1, 0])
    right /= np.linalg.norm(right)
    up = np.cross(right, forward)
    shore = np.array([0.48197104, -0.86880293, 0.11351380])
    onshore = np.array([right @ shore, -up @ shore])
    onshore /= np.linalg.norm(onshore)
    paths = sorted((root / "screenshots").glob("capture-*.png"))
    assert len(paths) == 8, "requires the complete eight-capture replay"
    images = [np.array(Image.open(p).convert("L"), dtype=float)[80:-80, 80:-80] for p in paths]
    height, width = images[0].shape
    window = np.outer(np.hanning(height), np.hanning(width))
    pairs = []
    for first, second in zip(images, images[1:]):
        first = (first - first.mean()) * window
        second = (second - second.mean()) * window
        correlation = np.fft.fftshift(
            np.fft.ifft2(np.conj(np.fft.fft2(first)) * np.fft.fft2(second)).real
        )
        limit = 80
        search = correlation[
            height // 2 - limit : height // 2 + limit + 1,
            width // 2 - limit : width // 2 + limit + 1,
        ]
        y, x = np.unravel_index(search.argmax(), search.shape)
        shift = np.array([x - limit, y - limit])
        assert np.linalg.norm(shift) > 0, "no resolved motion"
        assert np.max(np.abs(shift)) < limit, "motion exceeds search window"
        confidence = float(search.max() / np.sqrt((first * first).sum() * (second * second).sum()))
        assert confidence > 0.9, "ambiguous pattern displacement"
        pairs.append({
            "dx": int(shift[0]), "dy": int(shift[1]), "correlation": confidence,
            "onshore_cosine": float(shift @ onshore / np.linalg.norm(shift)),
        })
    return {"run": str(root), "pairs": pairs}


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    before, after = [measure(Path(arg)) for arg in sys.argv[1:]]
    print(json.dumps({"disabled": before, "enabled": after}, indent=2))
    assert all(p["onshore_cosine"] < 0 for p in before["pairs"]), "offshore baseline not reproduced"
    assert all(p["onshore_cosine"] > 0 for p in after["pairs"]), "shoreward repair not demonstrated"
