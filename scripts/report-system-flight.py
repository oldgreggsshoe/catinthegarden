#!/usr/bin/env python3
"""Report sequential planet_to_moon replays; run with an awake display, no builds.

CATINGARDEN_SYSTEM_CONTROL=nearest removes only the more distant terrain body.
It retains the common atmosphere/cloud/star/post work, so the measured delta is
incremental distant terrain cost, not a standalone --body moon comparison.
"""
import argparse
import json
import math
import statistics
from pathlib import Path

CAPTURES = (0, 4, 8, 18, 27, 38, 48, 54)
PHASES = ("planet_surface", "ascent", "transfer", "descent", "moon_surface")


def report(directory):
    directory = Path(directory)
    manifest = json.loads((directory / "manifest.json").read_text())
    if manifest.get("passed") is not True:
        raise ValueError(f"{directory}: replay did not pass")
    frames = []
    for line in (directory / "log.jsonl").read_text().splitlines():
        fields = json.loads(line)["fields"]
        if fields.get("message") == "two body frame":
            if not math.isfinite(fields["wall_ms"]) or fields["wall_ms"] <= 0:
                raise ValueError("invalid frame time")
            frames.append(fields)
    if len(frames) != 3241:
        raise ValueError(f"expected 3241 measured frames, got {len(frames)}")
    print(f"\n{directory} (nearest terrain only: {frames[0]['nearest_only']})")
    print("phase             frames median_ms   p95_ms   max_ms   FPS  max planet/moon chunks")
    for phase in PHASES:
        # PNG readback/compression is synchronous. Exclude its adjacent frames
        # equally from both runs, never trim ordinary streaming stalls.
        samples = [f for f in frames if f["phase"] == phase
                   and all(abs(f["time"] - t) > 0.1 for t in CAPTURES)]
        times = sorted(f["wall_ms"] for f in samples)
        median = statistics.median(times)
        p95 = times[math.ceil(0.95 * len(times)) - 1]
        print(f"{phase:17} {len(times):5} {median:9.3f} {p95:8.3f} {max(times):8.3f} "
              f"{1000/median:5.1f}  {max(f['planet_chunks'] for f in samples):3}/"
              f"{max(f['moon_chunks'] for f in samples):3}")
    print("Final moon clearance:", frames[-1]["moon_clearance"])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runs", nargs="+", help="test-runs/planet_to_moon/<run-id>")
    args = parser.parse_args()
    for run in args.runs:
        report(run)
