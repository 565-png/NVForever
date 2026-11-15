"""BC5 field scenario setup and outcome boundaries; no game or GPU is launched."""
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(HERE))

from smoke.plan import plan  # noqa: E402
from smoke.runner import Env, judge, run_launch  # noqa: E402
from smoke.scenarios import load  # noqa: E402


class Bc5FieldSetupTests(unittest.TestCase):
    def scenario(self):
        suite = load(HERE / "scenarios.toml")
        return suite, suite.by_name()["bc5-signed-normal-owned-field"]

    def test_actual_game_launch_supplies_explicit_aid_and_owned_relative_home(self):
        suite, scenario = self.scenario()
        launch, = plan(suite, [scenario])
        with tempfile.TemporaryDirectory() as d:
            out = Path(d)
            child = out / "record.py"
            child.write_text(
                "import json, os\nfrom pathlib import Path\n"
                "print(json.dumps({'aid': os.environ.get('NVF_PLAY_TEST_BC5_FIELD'), "
                "'home': str(Path(os.environ['NVFOREVER_HOME']).resolve()), 'cwd': os.getcwd()}))\n",
                encoding="utf-8")
            if os.name == "nt":
                exe = out / "record.cmd"
                exe.write_text(f'@"{sys.executable}" "{child}"\n', encoding="utf-8")
            else:
                exe = out / "record.sh"
                exe.write_text(f'#!/bin/sh\nexec "{sys.executable}" "{child}"\n', encoding="utf-8")
                exe.chmod(0o755)
            code, error, _ = run_launch(launch, Env(repo=HERE.parent.parent, out=out, exe=exe, data="unused-authored-bootstrap-placeholder", fnvtool=None), None)
            self.assertEqual((code, error), (0, ""))
            actual = json.loads((out / f"{launch.name}.log").read_text(encoding="utf-8"))
            self.assertEqual(actual["aid"], "1")
            self.assertEqual(Path(actual["home"]), out / "bc5-field-home")
            self.assertEqual(Path(actual["cwd"]), out)
            # The recorder only observes the actual process environment. It does
            # not claim that the Rust aid initialized or any frame was rendered.
            self.assertFalse((out / "bc5-field-home").exists())

    def test_success_needs_both_readbacks_and_rejects_shader_errors(self):
        _, scenario = self.scenario()
        markers = [
            "bc5 field: actual NIF material source, 3 authored panels, 4 quadrants",
            "bc5 field: first rendered readback PASS",
            "bc5 field: second rendered readback PASS",
            "bc5 field: rendered comparison PASS",
            "run finished",
        ]
        self.assertEqual(judge(scenario, markers, 0).status, "PASS")
        for i in range(4):
            self.assertEqual(judge(scenario, markers[:i] + markers[i + 1:], 0).status, "FAIL")
        self.assertEqual(judge(scenario, markers + ["wgpu error: validation error"], 0).status, "FAIL")
        self.assertEqual(judge(scenario, markers + ["failed to create shader module: malformed WGSL"], 0).status, "FAIL")
        self.assertEqual(judge(scenario, markers + ["bc5 field: FAIL readback 2"], 0).status, "FAIL")
        self.assertEqual(judge(scenario, markers, 1).status, "FAIL")
        self.assertEqual(judge(scenario, markers[:1] + [markers[2], markers[1]] + markers[3:], 0).status, "FAIL")


if __name__ == "__main__":
    unittest.main()
