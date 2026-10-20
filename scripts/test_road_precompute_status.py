"""Run with python3 -B -m unittest discover -s scripts -p 'test_road_precompute_status.py'."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


spec = importlib.util.spec_from_file_location(
    "road_precompute_status", Path(__file__).with_name("road-precompute-status.py")
)
status = importlib.util.module_from_spec(spec)
spec.loader.exec_module(status)


class RoadPrecomputeStatusTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def write(self, name, state="ok", bores=None, **fields):
        record = dict(key=name, status=state, source_version=11553600317892695948,
                      bores=bores or [])
        record.update(fields)
        path = self.root / (name + ".json")
        path.write_text(json.dumps(record))
        return path

    def test_counts_failures_and_partial_progress_without_changing_files(self):
        self.write("PositiveX_8_1_2", bores=[dict(length_meters=50, undersea_length_meters=0)])
        self.write("PositiveX_8_2_2", "failed", error='Rejected { reason: "budget exceeded" }')
        (self.root / "summary.json").write_text("old summary")
        (self.root / "PositiveX_8_3_2.tmp").write_text("not yet published")
        before = {p.name: p.read_bytes() for p in self.root.iterdir()}
        result = status.report(self.root, 10)
        self.assertEqual((result["regional_files"], result["succeeded"], result["failed"]), (2, 1, 1))
        self.assertEqual(result["processed_percent"], 20)
        self.assertEqual(result["failure_reasons"], {"budget exceeded": 1})
        self.assertEqual(result["files_with_plans"], 0)
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.root.iterdir()})

    def test_reports_recorded_components_not_regional_totals_or_failed_lengths(self):
        self.write("PositiveX_8_1_2", bores=[dict(length_meters=200, undersea_length_meters=10),
                                             dict(length_meters=150, undersea_length_meters=100)],
                   tunnel_length_meters=350, plan=[{}, {}, []])
        self.write("NegativeZ_8_1_2", "failed", bores=[dict(length_meters=999, undersea_length_meters=999)])
        result = status.report(self.root)
        self.assertEqual(result["longest_covered_component"]["length_meters"], 200)
        self.assertEqual(result["longest_undersea_component_portion"]["length_meters"], 100)
        self.assertEqual(result["files_with_plans"], 1)
        self.assertEqual(result["components_with_undersea_sections"], 2)
        self.assertEqual(result["wholly_undersea_components"], 0)
        self.assertEqual(result["regions_with_undersea_components"], 1)
        self.assertIn("including branches", result["note"])

    def test_invalid_results_are_visible_instead_of_silently_counted_as_success(self):
        path = self.write("PositiveX_8_1_2")
        path.write_text("{truncated")
        self.write("PositiveX_8_2_2", bores=[dict(length_meters=-1, undersea_length_meters=0)])
        self.write("PositiveX_8_3_2", key="wrong")
        self.write("PositiveX_8_4_2", bores=[dict(length_meters=float("nan"), undersea_length_meters=0)])
        result = status.report(self.root)
        self.assertEqual(len(result["invalid_files"]), 4)
        self.assertEqual(result["succeeded"], 0)

    def test_empty_output_and_multiple_terrain_versions(self):
        result = status.report(self.root)
        self.assertEqual(result["regional_files"], 0)
        self.assertIsNone(result["longest_covered_component"])
        self.write("PositiveX_8_1_2", source_version=1)
        self.write("PositiveX_8_2_2", source_version=2)
        result = status.report(self.root)
        self.assertEqual(result["source_versions"], {"1": 1, "2": 1})


if __name__ == "__main__":
    unittest.main()
