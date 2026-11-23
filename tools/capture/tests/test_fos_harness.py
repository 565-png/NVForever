"""Bridge so `unittest discover -s tools/capture/tests` runs the offline `.fos` synthetic suite."""

from pathlib import Path
import sys

FOS_TESTS_DIR = Path(__file__).resolve().parent.parent / "fos" / "tests"
if str(FOS_TESTS_DIR) not in sys.path:
    sys.path.insert(0, str(FOS_TESTS_DIR))

from test_parser_synthetic import (  # noqa: E402,F401
    TestCampaignsSchemaAndHygiene,
    TestCursorAndPrimitives,
    TestGlobalsAndChangedForms,
    TestHarnessBackupEspAndStaging,
)
