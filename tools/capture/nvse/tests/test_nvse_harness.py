"""Offline stdlib unittest suite for `tools/capture/nvse/` (no game install required)."""

from __future__ import annotations

from contextlib import redirect_stdout
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest

NVSE_DIR = Path(__file__).resolve().parent.parent
if str(NVSE_DIR) not in sys.path:
    sys.path.insert(0, str(NVSE_DIR))

import harness as nvse_harness  # noqa: E402


def _snapshot_tree(root: Path) -> dict[str, str]:
    snap: dict[str, str] = {}
    for p in sorted(root.rglob("*")):
        rel = p.relative_to(root).as_posix()
        snap[rel] = "DIR" if p.is_dir() else hashlib.sha256(p.read_bytes()).hexdigest()
    return snap


def _minimal_probe_catalog(
    *,
    probe_id: str = "demo_probe_one",
    spec_file: str = "docs/spec-notes/ext-showoff.md",
    script_lines: list[str] | None = None,
    match: dict[str, str] | None = None,
    fixtures: list[dict[str, str]] | None = None,
) -> dict:
    probe: dict = {
        "id": probe_id,
        "bucket": "A",
        "extender": "ShowOffNVSE 184",
        "required_plugins": ["jip_nvse.dll", "ShowOffNVSE.dll"],
        "optional_extender": False,
        "requires_world": False,
        "mutates_state": False,
        "requires_fixture": bool(fixtures),
        "requires_isolated_staging": bool(fixtures),
        "spec_file": spec_file,
        "spec_line": 10,
        "functions_tested": ["Matrix_IsMatrix"],
        "claim": "Sample clean-room claim.",
        "expected": "1,0,0",
        "script_lines": script_lines
        or [
            "array_var aList = ar_List 1 2 3",
            f'PrintToConsole "PROBE|{probe_id}|%g" (Matrix_IsMatrix aList)',
        ],
    }
    if match is not None:
        probe["match"] = match
    if fixtures is not None:
        probe["fixtures"] = fixtures
    return {
        "schema": nvse_harness.PROBE_CATALOG_SCHEMA,
        "catalog_id": "demo_catalog",
        "description": "Synthetic test catalog.",
        "game_version": "Fallout: New Vegas 1.4.0.525 (040020D0)",
        "xnvse_version": "6.4.9",
        "official_masters": list(nvse_harness.OFFICIAL_MASTERS),
        "extender_versions": {"JIP LN NVSE": "57.30", "ShowOffNVSE Plugin": "184"},
        "batches": [
            {
                "id": "demo_batch",
                "title": "Demo Batch",
                "extender": "ShowOffNVSE 184",
                "required_plugins": ["jip_nvse.dll", "ShowOffNVSE.dll"],
                "probes": [probe],
            }
        ],
    }


class TestCatalogValidation(unittest.TestCase):
    def test_committed_catalogs_validate_cleanly(self) -> None:
        paths = nvse_harness.iter_catalog_paths(catalog_dir=nvse_harness.DEFAULT_CATALOG_DIR)
        errors, counts = nvse_harness.validate_catalogs(paths)
        self.assertEqual(errors, [], f"Catalog validation errors: {errors[:5]}")
        self.assertEqual(counts["catalogs"], 3)
        self.assertEqual(counts["probe_batches"], 11)
        self.assertEqual(counts["probes"], 363)
        self.assertEqual(counts["persistence_items"], 28)

    def test_rejects_bare_editorids_and_allows_quoted_or_commented(self) -> None:
        bad_cat = _minimal_probe_catalog(
            script_lines=[
                "Player.AddItem Weap10mmPistol 1 1",
                "ref rD = DocMitchellRef",
                'PrintToConsole "PROBE|demo_probe_one|ok"',
            ]
        )
        errs = nvse_harness.validate_catalog_data(bad_cat, repo_root=None)
        self.assertTrue(any("bare EditorID" in e for e in errs), errs)

        ok_cat = _minimal_probe_catalog(
            script_lines=[
                "; Weap10mmPistol comment is ignored",
                'ref rG = GetFormFromMod "FalloutNV.esm" "434F"',
                'string_var sNote = "DocMitchellRef in string literal is ignored"',
                'PrintToConsole "PROBE|demo_probe_one|%i" rG',
            ]
        )
        self.assertEqual(nvse_harness.validate_catalog_data(ok_cat, repo_root=None), [])

    def test_rejects_write_string_to_file_in_catalog(self) -> None:
        bad_cat = _minimal_probe_catalog(
            script_lines=[
                'WriteStringToFile "nvseprobe_out.log" 1 "PROBE|demo_probe_one|1%r"',
            ]
        )
        errs = nvse_harness.validate_catalog_data(bad_cat, repo_root=None)
        self.assertTrue(any("WriteStringToFile" in e for e in errs), errs)

    def test_rejects_duplicate_ids_and_bad_spec_paths_and_privacy_leaks(self) -> None:
        cat1 = _minimal_probe_catalog(probe_id="dup_probe")
        cat2 = _minimal_probe_catalog(probe_id="dup_probe")
        seen: set[str] = set()
        self.assertEqual(nvse_harness.validate_catalog_data(cat1, seen_ids=seen), [])
        errs_dup = nvse_harness.validate_catalog_data(cat2, seen_ids=seen)
        self.assertTrue(any("duplicates another catalog" in e for e in errs_dup), errs_dup)

        bad_spec = _minimal_probe_catalog(spec_file="../outside.md")
        errs_spec = nvse_harness.validate_catalog_data(bad_spec, repo_root=None)
        self.assertTrue(any("clean relative POSIX path" in e for e in errs_spec), errs_spec)

        leaky = _minimal_probe_catalog()
        leaky["description"] = r"Captured from C:\Users\alice\FalloutNV"
        errs_leak = nvse_harness.validate_catalog_data(leaky, repo_root=None)
        self.assertTrue(any("privacy leak" in e for e in errs_leak), errs_leak)

    def test_cli_list_and_validate(self) -> None:
        buf = io.StringIO()
        with redirect_stdout(buf):
            rc_list = nvse_harness.main(["list", "--json"])
            rc_val = nvse_harness.main(["validate"])
        self.assertEqual(rc_list, 0)
        self.assertEqual(rc_val, 0)
        self.assertIn("wave4a_extenders", buf.getvalue())
        self.assertIn("persistence_matrix", buf.getvalue())


class TestEmitAndReadOnlyBoundary(unittest.TestCase):
    def test_no_auto_launcher_or_ini_patcher_present(self) -> None:
        self.assertFalse(hasattr(nvse_harness, "subprocess"))
        self.assertFalse(hasattr(nvse_harness, "patch_ini"))
        self.assertFalse(hasattr(nvse_harness, "run_probe_session"))

    def test_emit_console_mode_and_game_dir_boundary_guard(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tdp = Path(td)
            gdir = tdp / "game"
            sdir = tdp / "saves"
            (gdir / "Data").mkdir(parents=True)
            (gdir / "FalloutNV.exe").write_bytes(b"MZ-stub")
            (gdir / "Data" / "FalloutNV.esm").write_bytes(b"TES4-stub")
            sdir.mkdir()
            (sdir / "autosave.fos").write_bytes(b"FO3SAVEGAME-stub")

            game_before = _snapshot_tree(gdir)
            saves_before = _snapshot_tree(sdir)

            cat_path = tdp / "demo.json"
            cat_path.write_text(
                json.dumps(
                    _minimal_probe_catalog(
                        fixtures=[{"rel_path": "Data/Config/demo.ini", "content": "[Main]\na=1\n"}]
                    )
                ),
                encoding="utf-8",
            )

            # Reject --out-dir == --game-dir, inside --game-dir, ancestor of --game-dir, inside --saves-dir
            for bad_out in (
                gdir,
                gdir / "Data" / "NVSE",
                gdir / "sub" / ".." / "Data",
                tdp,
                sdir,
                sdir / "nested",
            ):
                with self.assertRaises(nvse_harness.ReadOnlyBoundaryError):
                    nvse_harness.emit_catalog(
                        cat_path,
                        bad_out,
                        game_dir=gdir,
                        saves_dir=sdir,
                    )

            # Even WITHOUT --game-dir, refuse a directory containing FalloutNV.exe / Data/FalloutNV.esm
            with self.assertRaises(nvse_harness.ReadOnlyBoundaryError):
                nvse_harness.emit_catalog(cat_path, gdir)

            # Valid emit into explicit clean out-dir (default console mode -> zero WriteStringToFile)
            out_console = tdp / "staged_console"
            info = nvse_harness.emit_catalog(
                cat_path,
                out_console,
                game_dir=gdir,
                saves_dir=sdir,
            )
            self.assertEqual(info["emitted_probes"], 1)
            self.assertEqual(info["output_mode"], "console")
            probe_gek = out_console / "Data" / "NVSE" / "user_defined_functions" / "nvseprobe_s_demo_probe_one.gek"
            loop_gek = out_console / "Data" / "NVSE" / "user_defined_functions" / "nvseprobe_loop.gek"
            gr_txt = out_console / "Data" / "NVSE" / "Plugins" / "Scripts" / "gr_nvseprobe.txt"
            setup_txt = out_console / "MANUAL_SETUP.txt"
            fixture_file = out_console / "fixtures" / "Data" / "Config" / "demo.ini"
            for p in (probe_gek, loop_gek, gr_txt, setup_txt, fixture_file):
                self.assertTrue(p.is_file(), f"Missing emitted file: {p}")
            for p in (probe_gek, loop_gek, gr_txt):
                txt = p.read_text(encoding="ascii")
                self.assertNotIn("WriteStringToFile", txt)

            # Opt-in file mode works with plain filename and rejects traversal
            with self.assertRaises(ValueError):
                nvse_harness.emit_catalog(
                    cat_path, tdp / "staged_badlog", output_mode="file", log_name="../escape.log"
                )
            out_file = tdp / "staged_file"
            nvse_harness.emit_catalog(
                cat_path,
                out_file,
                game_dir=gdir,
                saves_dir=sdir,
                output_mode="file",
                log_name="custom_probe.log",
            )
            file_probe_txt = (
                out_file / "Data" / "NVSE" / "user_defined_functions" / "nvseprobe_s_demo_probe_one.gek"
            ).read_text(encoding="ascii")
            self.assertIn('WriteStringToFile "custom_probe.log" 1 "PROBE|demo_probe_one|%g%r"', file_probe_txt)

            # Assert ZERO modifications anywhere in game_dir or saves_dir
            self.assertEqual(_snapshot_tree(gdir), game_before)
            self.assertEqual(_snapshot_tree(sdir), saves_before)

    def test_emit_persistence_catalog_two_boot_layout(self) -> None:
        pers_cat = nvse_harness.DEFAULT_CATALOG_DIR / "persistence_matrix.json"
        with tempfile.TemporaryDirectory() as td:
            out_dir = Path(td) / "staged_p2"
            info = nvse_harness.emit_catalog(pers_cat, out_dir)
            self.assertEqual(info["emitted_probes"], 27)
            self.assertTrue((out_dir / "boot1" / "Data" / "NVSE" / "user_defined_functions" / "nvseprobe_s_p2_mutate.gek").is_file())
            self.assertTrue((out_dir / "boot1" / "Data" / "NVSE" / "Plugins" / "Scripts" / "gl_nvseprobe_postload.txt").is_file())
            self.assertTrue((out_dir / "boot2" / "Data" / "NVSE" / "Plugins" / "Scripts" / "gl_nvseprobe_coldpost.txt").is_file())
            self.assertTrue((out_dir / "script_runner_prefixes" / "gx_nvseprobe_sr.txt").is_file())
            self.assertTrue((out_dir / "MANUAL_SETUP.txt").is_file())
            rb = (out_dir / "boot1" / "Data" / "NVSE" / "user_defined_functions" / "nvseprobe_s_p2_readback.gek").read_text(encoding="ascii")
            self.assertNotIn("WriteStringToFile", rb)

    def test_ancestor_sentinel_blocks_nested_game_and_ini_dirs_without_flags(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tdp = Path(td)
            gdir = tdp / "fnv_install"
            (gdir / "Data").mkdir(parents=True)
            (gdir / "FalloutNV.exe").write_bytes(b"MZ-stub")
            (gdir / "Data" / "FalloutNV.esm").write_bytes(b"TES4-stub")

            ini_dir = tdp / "my_games_fnv"
            (ini_dir / "Saves" / "profile1").mkdir(parents=True)
            (ini_dir / "Fallout.ini").write_bytes(b"[General]\r\n")

            cat_path = tdp / "demo.json"
            cat_path.write_text(json.dumps(_minimal_probe_catalog()), encoding="utf-8")

            game_before = _snapshot_tree(gdir)
            ini_before = _snapshot_tree(ini_dir)

            # Even when game_dir=None and saves_dir=None, nested paths under a live
            # game install or My Games\FalloutNV INI/Saves directory MUST be rejected.
            for nested_bad in (
                gdir / "Data" / "NVSE",
                gdir / "Data" / "NVSE" / "user_defined_functions",
                ini_dir / "Saves",
                ini_dir / "Saves" / "profile1" / "staged",
            ):
                with self.assertRaises(nvse_harness.ReadOnlyBoundaryError):
                    nvse_harness.emit_catalog(cat_path, nested_bad, game_dir=None, saves_dir=None)

            self.assertEqual(_snapshot_tree(gdir), game_before)
            self.assertEqual(_snapshot_tree(ini_dir), ini_before)

    def test_emit_rejects_path_traversal_and_unsafe_names(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tdp = Path(td)
            out_dir = tdp / "staged"

            # 1. Fixture rel_path traversal
            bad_fx = _minimal_probe_catalog(
                fixtures=[{"rel_path": "../../escape.ini", "content": "x=1\n"}]
            )
            bad_fx_path = tdp / "bad_fx.json"
            bad_fx_path.write_text(json.dumps(bad_fx), encoding="utf-8")
            with self.assertRaises(ValueError):
                nvse_harness.emit_catalog(bad_fx_path, out_dir)
            self.assertFalse((tdp / "escape.ini").exists())

            # 2. Helper script filename traversal
            bad_hlp = _minimal_probe_catalog()
            bad_hlp["batches"][0]["probes"][0]["helper_scripts"] = {
                "../../escape.gek": ["int i = 1"]
            }
            bad_hlp_path = tdp / "bad_hlp.json"
            bad_hlp_path.write_text(json.dumps(bad_hlp), encoding="utf-8")
            with self.assertRaises(ValueError):
                nvse_harness.emit_catalog(bad_hlp_path, out_dir)
            self.assertFalse((tdp / "escape.gek").exists())

            # 3. Unsafe probe id
            bad_id = _minimal_probe_catalog(probe_id="../escape_probe")
            bad_id_path = tdp / "bad_id.json"
            bad_id_path.write_text(json.dumps(bad_id), encoding="utf-8")
            with self.assertRaises(ValueError):
                nvse_harness.emit_catalog(bad_id_path, out_dir)

            # 4. Unsafe probe_save_name in persistence catalog
            pers_cat = nvse_harness.load_catalog(
                nvse_harness.DEFAULT_CATALOG_DIR / "persistence_matrix.json"
            )
            pers_cat["probe_save_name"] = "../escape_save"
            bad_pers_path = tdp / "bad_pers.json"
            bad_pers_path.write_text(json.dumps(pers_cat), encoding="utf-8")
            with self.assertRaises(ValueError):
                nvse_harness.emit_catalog(bad_pers_path, out_dir)

    def test_emit_helper_script_does_not_duplicate_begin_function_header(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tdp = Path(td)
            cat = _minimal_probe_catalog()
            cat["batches"][0]["probes"][0]["helper_scripts"] = {
                "nvseprobe_s_hlp_noargs.gek": [
                    "begin Function {}",
                    'Player.AuxVarSetFlt "*x" 1',
                ],
                "nvseprobe_s_hlp_params.gek": [
                    "begin Function { ref rRef }",
                    'Player.AuxVarSetRef "*r" rRef',
                ],
                "nvseprobe_s_hlp_bare.gek": [
                    'Player.AuxVarSetFlt "*y" 2',
                ],
            }
            cat_path = tdp / "helpers.json"
            cat_path.write_text(json.dumps(cat), encoding="utf-8")
            out_dir = tdp / "staged"
            nvse_harness.emit_catalog(cat_path, out_dir)

            udf = out_dir / "Data" / "NVSE" / "user_defined_functions"
            noargs_txt = (udf / "nvseprobe_s_hlp_noargs.gek").read_text(encoding="ascii")
            params_txt = (udf / "nvseprobe_s_hlp_params.gek").read_text(encoding="ascii")
            bare_txt = (udf / "nvseprobe_s_hlp_bare.gek").read_text(encoding="ascii")

            self.assertEqual(noargs_txt.lower().count("begin function"), 1, noargs_txt)
            self.assertEqual(params_txt.lower().count("begin function"), 1, params_txt)
            self.assertIn("begin Function { ref rRef }", params_txt)
            self.assertEqual(bare_txt.lower().count("begin function"), 1, bare_txt)
            self.assertIn("begin Function {}", bare_txt)


class TestPersistenceClassifierAndVerify(unittest.TestCase):
    def test_classify_persistence_tier_all_states(self) -> None:
        self.assertEqual(nvse_harness.classify_persistence_tier("123.5", "123.5", "123.5", baseline="0.0"), "SAVED")
        self.assertEqual(nvse_harness.classify_persistence_tier("456.5", "456.5", "0.0", baseline="0.0"), "SESSION_ONLY")
        self.assertEqual(nvse_harness.classify_persistence_tier("1", "0", "0", baseline="0"), "CLEARED_ON_LOAD")
        self.assertEqual(nvse_harness.classify_persistence_tier("0", "0", "0", baseline="0"), "UNCHANGED")
        self.assertEqual(nvse_harness.classify_persistence_tier("1", "1", None, baseline="0"), "INCOMPLETE")
        self.assertEqual(nvse_harness.classify_persistence_tier("1", "0", "1", baseline="0"), "ANOMALOUS")
        # Multi-field extraction
        self.assertEqual(
            nvse_harness.classify_persistence_tier(
                "perm=123.5,temp=456.5",
                "perm=123.5,temp=456.5",
                "perm=123.5,temp=0.0",
                baseline="0.0",
                field="perm",
            ),
            "SAVED",
        )
        self.assertEqual(
            nvse_harness.classify_persistence_tier(
                "perm=123.5,temp=456.5",
                "perm=123.5,temp=456.5",
                "perm=123.5,temp=0.0",
                baseline="0.0",
                field="temp",
            ),
            "SESSION_ONLY",
        )

    def test_verify_probes_and_two_boot_persistence_read_only(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tdp = Path(td)
            log_dir = tdp / "logs"
            out_dir = tdp / "reports"
            gdir = tdp / "game"
            log_dir.mkdir()
            gdir.mkdir()
            (gdir / "FalloutNV.exe").write_bytes(b"MZ-stub")

            cat_path = tdp / "probe_cat.json"
            cat_path.write_text(
                json.dumps(
                    _minimal_probe_catalog(
                        probe_id="so_01_mat_is_stringmap",
                        match={"type": "exact", "value": "1,0,0"},
                    )
                ),
                encoding="utf-8",
            )
            (log_dir / "run1.log").write_text(
                "\n".join(
                    [
                        "[12:00:00] PROBE|so_01_mat_is_stringmap|1,0,0",
                        "SR|boot1|gr_fired",
                        "SR|boot1|ln_fired",
                        "SR|boot1|gn_fired",
                        "P2|BEFORE|auxvar|perm=123.5,temp=456.5",
                        "P2|SAMERUN|auxvar|perm=123.5,temp=456.5",
                        "P2|COLD|auxvar|perm=123.5,temp=0.0",
                        "P2|BEFORE|combat_disabled|1",
                        "P2|SAMERUN|combat_disabled|0",
                        "P2|COLD|combat_disabled|0",
                        "P2|BEFORE|persistent|ref=FF001321,pers=1",
                        "P2|SAMERUN|persistent|ref=FF001321,pers=1",
                        "P2|COLD|persistent|ref=FF001321,pers=1",
                        "P2|BEFORE|hide_barter|0",
                        "P2|SAMERUN|hide_barter|0",
                        "P2|COLD|hide_barter|0",
                        "P2|BEFORE|sup_save_meta|sav=[],ld=[]",
                        "P2|SAMERUN|sup_save_meta|sav=[nvseprobe_p2.fos],ld=[nvseprobe_p2.fos]",
                        "P2|COLD|sup_save_meta|sav=[],ld=[nvseprobe_p2.fos]",
                    ]
                )
                + "\n",
                encoding="utf-8",
            )
            logs_before = _snapshot_tree(log_dir)

            # Without --redact-save-names, raw nvseprobe_p2.fos fails the privacy gate!
            pers_cat = nvse_harness.DEFAULT_CATALOG_DIR / "persistence_matrix.json"
            with self.assertRaises(nvse_harness.PrivacyLeakError):
                nvse_harness.verify_logs([cat_path, pers_cat], log_dir=log_dir, redact_saves=False)

            # With --redact-save-names, privacy gate passes and tiers classify accurately!
            sb = nvse_harness.verify_logs([cat_path, pers_cat], log_dir=log_dir, redact_saves=True)
            self.assertEqual(sb["counts"]["match"], 1)
            by_id = {p["id"]: p for p in sb["persistence"]}
            self.assertEqual(by_id["p2_auxvar_perm"]["observed_tier"], "SAVED")
            self.assertEqual(by_id["p2_auxvar_temp"]["observed_tier"], "SESSION_ONLY")
            self.assertEqual(by_id["p2_combat_disabled"]["observed_tier"], "CLEARED_ON_LOAD")
            self.assertEqual(by_id["p2_persistent"]["observed_tier"], "SAVED")
            self.assertEqual(by_id["p2_hide_barter"]["observed_tier"], "UNCHANGED")
            self.assertIn("<save>.fos", by_id["p2_sup_save_meta"]["samerun"])
            self.assertEqual(sb["script_runner"]["boot1"], ["gr_fired", "ln_fired", "gn_fired"])

            # Verify CLI refuses writing --json-out inside --game-dir
            with self.assertRaises(nvse_harness.ReadOnlyBoundaryError):
                nvse_harness.main(
                    [
                        "verify",
                        "--catalog",
                        str(cat_path),
                        "--log-dir",
                        str(log_dir),
                        "--game-dir",
                        str(gdir),
                        "--json-out",
                        str(gdir / "bad.json"),
                    ]
                )

            # Verify CLI writes clean JSON + MD into out_dir while leaving log_dir untouched
            json_out = out_dir / "score.json"
            md_out = out_dir / "score.md"
            rc = nvse_harness.main(
                [
                    "verify",
                    "--catalog",
                    str(cat_path),
                    "--log-dir",
                    str(log_dir),
                    "--game-dir",
                    str(gdir),
                    "--json-out",
                    str(json_out),
                    "--md-out",
                    str(md_out),
                    "--strict",
                ]
            )
            self.assertEqual(rc, 0)
            self.assertTrue(json_out.is_file())
            self.assertTrue(md_out.is_file())
            self.assertEqual(_snapshot_tree(log_dir), logs_before)


class TestCitationAndPrivacyGate(unittest.TestCase):
    def test_privacy_gate_catches_all_leak_classes(self) -> None:
        leaky_samples = [
            r"Path C:\Users\alice\Documents\log.txt",
            "Path D:/ExampleInstall/Game/Data",
            "Path /home/bob/fnv/log.txt",
            "Path /Users/bob/fnv/log.txt",
            r"UNC \\fileserver\share\save",
            "Save personal_run_01.fos loaded",
            "Symbol FalloutNV.pdb leaked",
            "Control\x07bell",
        ]
        for sample in leaky_samples:
            issues = nvse_harness.check_privacy_leaks(sample)
            self.assertTrue(issues, f"Expected privacy leak to be flagged for: {sample!r}")

        clean_sample = (
            "[observed: Fallout: New Vegas 1.4.0.525 + xNVSE 6.4.9 + ShowOffNVSE 184 "
            "with the 10 official base masters, clean-room black-box script probe `so_01` "
            "(Matrix_IsMatrix): sav=[<save>.fos], 1,0,0; no binary, DLL, or source consulted]"
        )
        self.assertEqual(nvse_harness.check_privacy_leaks(clean_sample), [])

    def test_citation_formatting_and_cli_cite(self) -> None:
        meta = {
            "game_version": "Fallout: New Vegas 1.4.0.525 (040020D0)",
            "xnvse_version": "6.4.9",
        }
        entry = {
            "id": "so_01_mat_is_stringmap",
            "extender": "ShowOffNVSE 184",
            "spec_file": "docs/spec-notes/ext-showoff.md",
            "spec_line": 216,
            "functions_tested": ["Matrix_IsMatrix"],
            "status": "MATCH",
            "observed": "1,0,0",
        }
        cite = nvse_harness.format_citation(entry, meta, is_persistence=False)
        self.assertIn("Fallout: New Vegas 1.4.0.525", cite)
        self.assertIn("xNVSE 6.4.9", cite)
        self.assertIn("10 official base masters", cite)
        self.assertIn("no binary, DLL, or source consulted", cite)

        p_entry = {
            "id": "p2_combat_disabled",
            "extender": "xNVSE 6.4.9 core",
            "spec_file": "docs/spec-notes/ext-last-core.md",
            "spec_line": 175,
            "functions_tested": ["SetCombatDisabled", "GetCombatDisabled"],
            "before": "1",
            "samerun": "0",
            "cold": "0",
            "observed_tier": "CLEARED_ON_LOAD",
            "verdict": "MATCH",
        }
        p_cite = nvse_harness.format_citation(p_entry, meta, is_persistence=True)
        self.assertIn("CLEARED_ON_LOAD", p_cite)
        self.assertIn("BEFORE=1, SAMERUN=0, COLD=0", p_cite)

        # CLI cite rejects leaky observations
        bad_entry = dict(entry, observed=r"C:\Users\alice\leak.txt")
        with self.assertRaises(nvse_harness.PrivacyLeakError):
            nvse_harness.format_citation(bad_entry, meta, is_persistence=False)

        with tempfile.TemporaryDirectory() as td:
            sb_path = Path(td) / "sb.json"
            sb_path.write_text(
                json.dumps({"metadata": meta, "probes": [entry], "persistence": [p_entry]}),
                encoding="utf-8",
            )
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc = nvse_harness.main(["cite", "--scoreboard", str(sb_path)])
            self.assertEqual(rc, 0)
            out = buf.getvalue()
            self.assertIn("so_01_mat_is_stringmap", out)
            self.assertIn("p2_combat_disabled", out)


if __name__ == "__main__":
    unittest.main()
