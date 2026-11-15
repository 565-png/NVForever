"""[design, #2652] Optional DLC gating using authored paths, never installed data."""
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from smoke.plan import Launch
from smoke.runner import missing_area_plugins
from smoke.scenarios import Scenario


class AreaPlugins(unittest.TestCase):
    def launch(self, args=(), kind="game"):
        return Launch("silo", kind, [Scenario("silo", areas=["lonesome-road"])], list(args))

    def test_missing_default_master_skips_but_available_master_runs(self):
        with tempfile.TemporaryDirectory() as folder:
            self.assertEqual(missing_area_plugins(self.launch(), folder), ["LonesomeRoad.esm"])
            self.assertEqual(missing_area_plugins(self.launch(["--plugins", "LonesomeRoad.esm"]), folder), ["LonesomeRoad.esm"])
            (Path(folder) / "LonesomeRoad.esm").touch()
            self.assertEqual(missing_area_plugins(self.launch(), folder), [])

    def test_explicit_plugin_selection_decides_even_when_master_is_available(self):
        with tempfile.TemporaryDirectory() as folder:
            (Path(folder) / "LonesomeRoad.esm").touch()
            self.assertEqual(missing_area_plugins(self.launch(["--plugins", "FalloutNV.esm"]), folder), ["LonesomeRoad.esm"])
            self.assertEqual(missing_area_plugins(self.launch(["--plugins", "FalloutNV.esm", "lonesomeroad.esm", "--play"]), folder), [])

    def test_tool_and_unrelated_scenarios_are_unaffected(self):
        self.assertEqual(missing_area_plugins(self.launch(kind="tool"), None), [])
        self.assertEqual(missing_area_plugins(Launch("base", "game", [Scenario("base", areas=["combat"])]), None), [])
