#!/usr/bin/env python3
"""Read a running road precompute's published results without modifying them.

Lengths are connected covered components within ONE region, including branches;
they are not necessarily entrance-to-exit tunnel lengths or global maxima.
"""

import argparse
from collections import Counter
from datetime import datetime, timezone
import json
import math
from pathlib import Path
import re


REGION_FILE = re.compile(r"(?:Positive|Negative)[XYZ]_\d+_\d+_\d+\.json\Z")
NOTE = (
    "Lengths sum connected covered segments within one region, including branches. "
    "Region boundaries split components; these are not necessarily entrance-to-exit "
    "tunnel lengths. Undersea figures count only the covered portions over open sea. "
    "A processed region may have failed; statistics-only files are not runtime road plans."
)


def length(value):
    if type(value) not in (int, float) or not math.isfinite(value) or value < 0:
        raise ValueError("invalid component length")
    return value


def report(directory, expected_regions=None):
    paths = sorted(p for p in directory.iterdir() if REGION_FILE.fullmatch(p.name))
    result = {
        "observed_at_utc": datetime.now(timezone.utc).isoformat(),
        "regional_files": len(paths),
        "succeeded": 0,
        "failed": 0,
        "invalid_files": [],
        "files_with_plans": 0,
        "source_versions": Counter(),
        "failure_reasons": Counter(),
        "failed_regions": [],
        "components_with_undersea_sections": 0,
        "wholly_undersea_components": 0,
        "regions_with_undersea_components": 0,
        "longest_covered_component": None,
        "longest_undersea_component_portion": None,
        "note": NOTE,
    }
    if expected_regions is not None:
        result["expected_regions"] = expected_regions
        result["processed_percent"] = len(paths) * 100.0 / expected_regions
    for path in paths:
        try:
            record = json.loads(path.read_text())
            if record["key"] != path.stem or record["status"] not in ("ok", "failed"):
                raise ValueError("unrecognised region record")
            version = record["source_version"]
            if type(version) is not int:
                raise ValueError("invalid terrain version")
            bores = record["bores"]
            if not isinstance(bores, list):
                raise ValueError("invalid component list")
            # Validate before counting anything, so malformed records are not
            # silently included among successes or reported as zero-length.
            for bore in bores:
                length(bore["length_meters"])
                length(bore["undersea_length_meters"])
        except (OSError, ValueError, KeyError, TypeError) as error:
            result["invalid_files"].append({"file": path.name, "error": str(error)})
            continue
        result["source_versions"][str(version)] += 1
        if record.get("plan") is not None:
            result["files_with_plans"] += 1
        if record["status"] == "failed":
            result["failed"] += 1
            result["failed_regions"].append(path.stem)
            error = str(record.get("error", "unspecified failure"))
            reasons = re.findall(r'reason: "([^"]+)"', error)
            result["failure_reasons"][reasons[-1] if reasons else error] += 1
            continue
        result["succeeded"] += 1
        if any(bore["undersea_length_meters"] > 0 for bore in bores):
            result["regions_with_undersea_components"] += 1
        for bore in bores:
            if bore["undersea_length_meters"] > 0:
                result["components_with_undersea_sections"] += 1
                if math.isclose(bore["length_meters"], bore["undersea_length_meters"], abs_tol=0.001):
                    result["wholly_undersea_components"] += 1
            for field, target in [
                ("length_meters", "longest_covered_component"),
                ("undersea_length_meters", "longest_undersea_component_portion"),
            ]:
                meters = bore[field]
                previous = result[target]
                if meters > 0 and (previous is None or meters > previous["length_meters"]):
                    result[target] = {"region": path.stem, "length_meters": meters}
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--expected-regions", type=int)
    parser.add_argument("--json", action="store_true", help="print the complete report as JSON")
    args = parser.parse_args()
    if args.expected_regions is not None and args.expected_regions <= 0:
        parser.error("--expected-regions must be positive")
    try:
        result = report(args.directory, args.expected_regions)
    except OSError as error:
        parser.error(str(error))
    if args.json:
        print(json.dumps(result, indent=2))
        return
    progress = f"{result['regional_files']:,} regional files"
    if "processed_percent" in result:
        progress += f" / {result['expected_regions']:,} ({result['processed_percent']:.2f}%)"
    print(progress)
    print(f"{result['succeeded']:,} successful; {result['failed']:,} failed; "
          f"{len(result['invalid_files']):,} invalid/unreadable")
    print(f"{result['files_with_plans']:,} files include road plans")
    for field, title in [
        ("longest_covered_component", "Largest regional covered component"),
        ("longest_undersea_component_portion", "Largest regional undersea portion"),
    ]:
        value = result[field]
        print(f"{title}: " + (f"{value['length_meters']:,.1f} m ({value['region']})"
                                 if value else "none recorded"))
    print(f"{result['components_with_undersea_sections']:,} regional components include undersea "
          f"sections; {result['wholly_undersea_components']:,} are wholly undersea")
    if len(result["source_versions"]) > 1:
        print("WARNING: results contain multiple terrain versions; totals mix different terrain.")
    for reason, count in result["failure_reasons"].most_common(8):
        print(f"  {count:,} failures: {reason}")
    print(NOTE)


if __name__ == "__main__":
    main()
