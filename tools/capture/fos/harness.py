#!/usr/bin/env python3
"""Parameterized clean-room black-box `.fos` A/B save-pair capture harness.

Safety guarantees:
- Zero hardcoded machine paths or GPU names: `--game-dir`, `--ini-dir`,
  `--loadorder-dir`, and `--out-dir` come strictly from CLI flags or env vars.
- Refuses to collide with an already-running `FalloutNV.exe` process.
- Terminates ONLY its own spawned PID tree (`taskkill /PID <pid> /T /F` on
  Windows, `SIGTERM`/`SIGKILL` on POSIX) — never kills unrelated instances.
- Captures SHA-256 + file mode + raw bytes of `Fallout.ini`, `FalloutPrefs.ini`,
  optional `FalloutCustom.ini`, `plugins.txt`, and `loadorder.txt`, and restores
  every protected file byte-for-byte in `try/finally` with SHA-256 verification.
- Isolates captures under `<ini-dir>/Saves/fosprobe/` and cleans that folder on
  entry and exit.
- Optional `--enable-synthetic-esp` builds a temporary TES4/FNV probe plugin
  (`fos_probe_r3.esp`) on-demand purely from public TES4/FNV binary record
  definitions and removes it in `try/finally`.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import shutil
import signal
import stat
import struct
import subprocess
import sys
import time
import zlib
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Iterable

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import campaigns as fos_campaigns  # type: ignore[import-not-found]
else:
    from . import campaigns as fos_campaigns

SYNTHETIC_ESP_FILENAME = "fos_probe_r3.esp"
PROBE_SAVE_SUBDIR = os.path.join("Saves", "fosprobe")


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: str | os.PathLike[str]) -> str:
    return sha256_bytes(Path(path).read_bytes())


def patch_ini(text: str, section: str, updates: dict[str, str]) -> str:
    """Update or insert `key=value` pairs under `[section]` while preserving order."""
    lines = text.splitlines(keepends=True)
    out: list[str] = []
    in_sec = False
    found_sec = False
    done: set[str] = set()
    sec_header = f"[{section}]".lower()

    for line in lines:
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            if in_sec:
                for k, v in updates.items():
                    if k.lower() not in done:
                        out.append(f"{k}={v}\r\n")
                        done.add(k.lower())
            in_sec = stripped.lower() == sec_header
            if in_sec:
                found_sec = True
            out.append(line)
            continue
        if in_sec and "=" in line and not stripped.startswith(";"):
            k = line.split("=", 1)[0].strip()
            for uk, uv in updates.items():
                if k.lower() == uk.lower():
                    if uk.lower() not in done:
                        out.append(f"{uk}={uv}\r\n")
                        done.add(uk.lower())
                    break
            else:
                out.append(line)
        else:
            out.append(line)

    if in_sec:
        for k, v in updates.items():
            if k.lower() not in done:
                out.append(f"{k}={v}\r\n")
                done.add(k.lower())

    if not found_sec and updates:
        if out and not out[-1].endswith("\n"):
            out.append("\r\n")
        out.append(f"[{section}]\r\n")
        for k, v in updates.items():
            out.append(f"{k}={v}\r\n")

    return "".join(out)


@dataclass
class _BackupRecord:
    path: Path
    existed: bool
    data: bytes | None
    mode: int | None
    digest: str | None


class FileBackupGuard:
    """Context manager that backs up files (bytes + mode + SHA-256) and restores
    them verbatim in `__exit__` even on exceptions. Files that did not exist
    before entering are removed on exit if created temporarily.
    """

    def __init__(self, paths: Iterable[str | os.PathLike[str]]) -> None:
        self._paths = [Path(p) for p in paths]
        self._records: list[_BackupRecord] = []

    def __enter__(self) -> "FileBackupGuard":
        self._records = []
        for path in self._paths:
            if path.exists():
                data = path.read_bytes()
                mode = path.stat().st_mode
                digest = sha256_bytes(data)
                self._records.append(_BackupRecord(path, True, data, mode, digest))
                os.chmod(path, mode | stat.S_IWRITE)
            else:
                self._records.append(_BackupRecord(path, False, None, None, None))
        return self

    def __exit__(self, exc_type, exc, tb) -> None:
        errors: list[str] = []
        for rec in self._records:
            try:
                if rec.existed:
                    assert rec.data is not None and rec.mode is not None and rec.digest is not None
                    if rec.path.exists():
                        os.chmod(rec.path, stat.S_IWRITE | stat.S_IREAD)
                    rec.path.write_bytes(rec.data)
                    os.chmod(rec.path, rec.mode)
                    restored_digest = sha256_file(rec.path)
                    if restored_digest != rec.digest:
                        errors.append(
                            f"SHA-256 mismatch restoring {rec.path}: "
                            f"expected {rec.digest}, got {restored_digest}"
                        )
                elif rec.path.exists():
                    os.chmod(rec.path, stat.S_IWRITE | stat.S_IREAD)
                    rec.path.unlink()
            except OSError as err:
                errors.append(f"Failed restoring {rec.path}: {err}")
        if errors:
            raise RuntimeError("; ".join(errors))


def default_is_fnv_running() -> bool:
    """Return True if any `FalloutNV.exe` process is currently running."""
    if os.name == "nt":
        res = subprocess.run(
            ["tasklist", "/FI", "IMAGENAME eq FalloutNV.exe", "/NH"],
            capture_output=True,
            text=True,
            check=False,
        )
        return "falloutnv.exe" in res.stdout.lower()
    res = subprocess.run(["pgrep", "-i", "FalloutNV"], capture_output=True, text=True, check=False)
    return res.returncode == 0


def check_no_running_fnv(
    *,
    is_running: Callable[[], bool] = default_is_fnv_running,
    wait_seconds: float = 0.0,
    poll_interval: float = 5.0,
) -> None:
    """Refuse to launch if another `FalloutNV.exe` instance is already active."""
    deadline = time.monotonic() + max(0.0, wait_seconds)
    while is_running():
        if time.monotonic() >= deadline:
            raise RuntimeError(
                "Another FalloutNV.exe process is already running; refusing to collide."
            )
        time.sleep(min(poll_interval, max(0.1, deadline - time.monotonic())))


def terminate_pid_tree(pid: int | None) -> None:
    """Terminate ONLY the specified spawned PID (and its children)."""
    if not pid:
        return
    if os.name == "nt":
        subprocess.run(["taskkill", "/PID", str(pid), "/T", "/F"], capture_output=True, check=False)
    else:
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            return


# ---------------------------------------------------------------------------
# Clean-room synthetic TES4/FNV test-plugin builder (`--enable-synthetic-esp`)
# ---------------------------------------------------------------------------


def _subrec(sig: bytes, payload: bytes) -> bytes:
    if len(sig) != 4 or len(payload) > 0xFFFF:
        raise ValueError(f"invalid subrecord {sig!r} len={len(payload)}")
    return sig + struct.pack("<H", len(payload)) + payload


def _record(sig: bytes, fid: int, body: bytes, flags: int = 0, version: int = 15) -> bytes:
    if len(sig) != 4:
        raise ValueError(f"invalid record signature {sig!r}")
    return sig + struct.pack("<IIIIHH", len(body), flags, fid, 0, version, 0) + body


def _grup(label: bytes, gtype: int, content: bytes) -> bytes:
    if len(label) != 4:
        raise ValueError(f"invalid GRUP label {label!r}")
    total = 24 + len(content)
    return b"GRUP" + struct.pack("<I", total) + label + struct.pack("<III", gtype, 0, 0) + content


def _extract_esm_records(
    esm_path: str | os.PathLike[str], targets: set[tuple[bytes, int]]
) -> dict[tuple[bytes, int], tuple[int, bytes]]:
    data = Path(esm_path).read_bytes()
    found: dict[tuple[bytes, int], tuple[int, bytes]] = {}

    def walk(start: int, end: int) -> None:
        p = start
        while p + 24 <= end:
            sig = data[p : p + 4]
            sz = struct.unpack_from("<I", data, p + 4)[0]
            if sig == b"GRUP":
                if sz < 24 or p + sz > end:
                    raise ValueError(f"malformed GRUP size {sz} at {p:#x}")
                walk(p + 24, p + sz)
                p += sz
            else:
                flags, fid = struct.unpack_from("<II", data, p + 8)
                if (sig, fid) in targets:
                    body = data[p + 24 : p + 24 + sz]
                    if flags & 0x40000:
                        body = zlib.decompress(body[4:])
                    found[(sig, fid)] = (flags, body)
                p += 24 + sz

    walk(0, len(data))
    return found


def _iter_subrecs(body: bytes):
    p = 0
    ln = len(body)
    xxxx: int | None = None
    while p + 6 <= ln:
        sig = body[p : p + 4]
        length = struct.unpack_from("<H", body, p + 4)[0]
        p += 6
        if sig == b"XXXX":
            xxxx = struct.unpack_from("<I", body, p)[0]
            p += length
            continue
        if xxxx is not None:
            length = xxxx
            xxxx = None
        yield sig, body[p : p + length]
        p += length


def _rebuild_subrecs(subs: list[tuple[bytes, bytes]]) -> bytes:
    return b"".join(_subrec(sig, payload) for sig, payload in subs)


def build_synthetic_probe_esp(
    esm_path: str | os.PathLike[str],
    out_path: str | os.PathLike[str],
) -> Path:
    """Build the clean-room temporary probe plugin (`fos_probe_r3.esp`) on-demand.

    Uses only public TES4/FNV binary record structure and template records read
    from the user's own `FalloutNV.esm` to exercise the 6 rare item-extra flags
    absent in vanilla data (`0x17`, `0x1C`, `0x22`, `0x26`, `0x28`, `0x30`).
    """
    targets = {
        (b"LIGH", 0x00177ADD),
        (b"ARMO", 0x00020423),
        (b"ENCH", 0x001673BF),
        (b"WEAP", 0x000E3778),
        (b"WEAP", 0x00162C92),
        (b"CELL", 0x00103DF9),
        (b"REFR", 0x001059AE),
    }
    src = _extract_esm_records(esm_path, targets)
    missing = targets - set(src.keys())
    if missing:
        raise ValueError(f"ESM missing template records: {sorted(missing)}")

    plunger_pos = None
    for s, b in _iter_subrecs(src[(b"REFR", 0x001059AE)][1]):
        if s == b"DATA":
            plunger_pos = struct.unpack("<6f", b)
    if plunger_pos is None:
        raise ValueError("Template REFR 0x001059AE missing DATA subrecord")
    px, py, pz = plunger_pos[0], plunger_pos[1], plunger_pos[2]

    # 1. LIGH: ProbeCarryTorch (0x01000801, CanBeCarried=0x02, Time=120s)
    rec_ligh = _record(
        b"LIGH",
        0x01000801,
        _rebuild_subrecs(
            [
                (b"EDID", b"ProbeCarryTorch\x00"),
                (b"OBND", struct.pack("<6h", -8, -8, -8, 8, 8, 8)),
                (b"MODL", b"Clutter\\Pencil01.NIF\x00"),
                (b"FULL", b"Probe Torch\x00"),
                (
                    b"DATA",
                    struct.pack("<iIIIffIf", 120, 256, 0x00A5D9DA, 0x00000002, 1.0, 90.0, 10, 1.0),
                ),
                (b"FNAM", struct.pack("<f", 1.0)),
            ]
        ),
    )

    # 2. ARMO: four ring/hand BMDT variants for 0x17 negative-proof test
    def make_armo(fid: int, edid: str, full: str, bmdt_flags: int) -> bytes:
        subs = []
        for s, b in _iter_subrecs(src[(b"ARMO", 0x00020423)][1]):
            if s == b"EDID":
                subs.append((b"EDID", edid.encode("ascii") + b"\x00"))
            elif s == b"FULL":
                subs.append((b"FULL", full.encode("ascii") + b"\x00"))
            elif s == b"BMDT":
                subs.append((b"BMDT", struct.pack("<IB3x", bmdt_flags, 0)))
            else:
                subs.append((s, b))
        return _record(b"ARMO", fid, _rebuild_subrecs(subs))

    rec_armo_dual = make_armo(0x01000802, "ProbeRingDual", "Probe Ring Dual", (1 << 7) | (1 << 8))
    rec_armo_left = make_armo(0x01000803, "ProbeRingLeft", "Probe Ring Left", 1 << 8)
    rec_armo_right = make_armo(0x01000804, "ProbeRingRight", "Probe Ring Right", 1 << 7)
    rec_armo_hand = make_armo(0x01000808, "ProbeDualHand", "Probe Dual Hand", (1 << 3) | (1 << 4))

    # 3. ENCH: weapon enchantment with non-zero charge=500, cost=25
    ench_subs = []
    for s, b in _iter_subrecs(src[(b"ENCH", 0x001673BF)][1]):
        if s == b"EDID":
            ench_subs.append((b"EDID", b"ProbeChargedEnch\x00"))
        elif s == b"FULL":
            ench_subs.append((b"FULL", b"Probe Charged Ench\x00"))
        elif s == b"ENIT":
            ench_subs.append((b"ENIT", struct.pack("<IIII", 2, 500, 25, 0)))
        else:
            ench_subs.append((s, b))
    rec_ench = _record(b"ENCH", 0x01000805, _rebuild_subrecs(ench_subs))

    # 4. WEAP: charged pistol & knife referencing ProbeChargedEnch (0x01000805)
    def make_charged_weap(src_fid: int, new_fid: int, edid: str, full: str) -> bytes:
        subs = []
        inserted_eitm = False
        for s, b in _iter_subrecs(src[(b"WEAP", src_fid)][1]):
            if s == b"EDID":
                subs.append((b"EDID", edid.encode("ascii") + b"\x00"))
            elif s == b"FULL":
                subs.append((b"FULL", full.encode("ascii") + b"\x00"))
            elif s in (b"EITM", b"EAMT"):
                continue
            else:
                subs.append((s, b))
            if s == b"FULL" and not inserted_eitm:
                subs.append((b"EITM", struct.pack("<I", 0x01000805)))
                subs.append((b"EAMT", struct.pack("<H", 500)))
                inserted_eitm = True
        return _record(b"WEAP", new_fid, _rebuild_subrecs(subs))

    rec_weap_pistol = make_charged_weap(0x000E3778, 0x01000806, "ProbeChargedPistol", "Probe Charged 9mm")
    rec_weap_knife = make_charged_weap(0x00162C92, 0x01000807, "ProbeChargedKnife", "Probe Charged Knife")

    # 5. CELL 0x00103DF9 override + 6 placed REFRs (XOWN / XGLB / XSCL / XESP)
    cell_flags, cell_body = src[(b"CELL", 0x00103DF9)]
    rec_cell = _record(b"CELL", 0x00103DF9, cell_body, flags=cell_flags)

    def make_refr(
        fid: int,
        edid: str,
        base_fid: int,
        pos: tuple[float, float, float],
        *,
        flags: int = 0,
        xown: int | None = None,
        xglb: int | None = None,
        xscl: float | None = None,
        xesp: int | None = None,
    ) -> bytes:
        subs: list[tuple[bytes, bytes]] = [
            (b"EDID", edid.encode("ascii") + b"\x00"),
            (b"NAME", struct.pack("<I", base_fid)),
        ]
        if xown is not None:
            subs.append((b"XOWN", struct.pack("<I", xown)))
        if xglb is not None:
            subs.append((b"XGLB", struct.pack("<I", xglb)))
        if xesp is not None:
            subs.append((b"XESP", struct.pack("<II", xesp, 0)))
        if xscl is not None:
            subs.append((b"XSCL", struct.pack("<f", xscl)))
        subs.append((b"DATA", struct.pack("<6f", pos[0], pos[1], pos[2], 0.0, 0.0, 0.0)))
        return _record(b"REFR", fid, _rebuild_subrecs(subs), flags=flags)

    pers_refs = b"".join(
        [
            make_refr(
                0x01000810,
                "ProbePersistXGLB",
                0x0005B63C,
                (px + 10, py, pz + 5),
                flags=0x400,
                xown=0x00104C0C,
                xglb=0x00000038,
                xscl=1.5,
            ),
            make_refr(
                0x01000811,
                "ProbePersistXGLBOnly",
                0x000340A4,
                (px + 20, py, pz + 5),
                flags=0x400,
                xglb=0x00000038,
            ),
            make_refr(
                0x01000812,
                "ProbePersistOrigRef",
                0x000340B5,
                (px + 30, py, pz + 5),
                flags=0x400,
                xscl=1.75,
                xesp=0x00104C0F,
            ),
        ]
    )
    temp_refs = b"".join(
        [
            make_refr(
                0x01000813,
                "ProbeTempXGLB",
                0x0005B63C,
                (px + 40, py, pz + 5),
                flags=0x0,
                xown=0x00104C0C,
                xglb=0x00000038,
                xscl=1.5,
            ),
            make_refr(
                0x01000814,
                "ProbeTempScaled",
                0x000340A4,
                (px + 50, py, pz + 5),
                flags=0x0,
                xscl=1.75,
            ),
            make_refr(
                0x01000815,
                "ProbeTempTorchRef",
                0x01000801,
                (px + 60, py, pz + 5),
                flags=0x0,
            ),
        ]
    )

    cell_fid_bytes = struct.pack("<I", 0x00103DF9)
    cell_children = _grup(
        cell_fid_bytes,
        6,
        _grup(cell_fid_bytes, 8, pers_refs) + _grup(cell_fid_bytes, 9, temp_refs),
    )
    blk = 0x00103DF9 % 10
    subblk = (0x00103DF9 // 10) % 10
    grup_cell = _grup(
        b"CELL",
        0,
        _grup(
            struct.pack("<I", blk),
            2,
            _grup(struct.pack("<I", subblk), 3, rec_cell + cell_children),
        ),
    )

    top_groups = b"".join(
        [
            _grup(b"ENCH", 0, rec_ench),
            _grup(b"LIGH", 0, rec_ligh),
            _grup(b"ARMO", 0, rec_armo_dual + rec_armo_left + rec_armo_right + rec_armo_hand),
            _grup(b"WEAP", 0, rec_weap_pistol + rec_weap_knife),
            grup_cell,
        ]
    )

    # 15 records + 9 GRUPs = 24 entries in HEDR numRecords.
    num_recs = 15 + 9
    tes4_body = _rebuild_subrecs(
        [
            (b"HEDR", struct.pack("<fII", 1.34, num_recs, 0x00000820)),
            (b"CNAM", b"fos_probe_r3\x00"),
            (b"MAST", b"FalloutNV.esm\x00"),
            (b"DATA", struct.pack("<Q", 0)),
        ]
    )
    tes4 = _record(b"TES4", 0, tes4_body, flags=0)
    out = Path(out_path)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_bytes(tes4 + top_groups)
    return out


# ---------------------------------------------------------------------------
# Campaign staging & execution
# ---------------------------------------------------------------------------


STAGED_STEP_PREFIX = "fosprobe_s_"
STAGED_LOOP_FILENAME = "fosprobe_loop.gek"
STAGED_GR_PREFIX = "gr_fosprobe"


def cleanup_staged_scripts(game_dir: str | os.PathLike[str]) -> None:
    """Remove ONLY harness-owned staged scripts (`gr_fosprobe*`, `fosprobe_loop.gek`,
    and `fosprobe_s_*`) from `<game-dir>/Data/NVSE/`. Never touches unrelated user/mod
    files (such as `s_*.gek`).
    """
    gdir = Path(game_dir)
    scripts_dir = gdir / "Data" / "NVSE" / "Plugins" / "Scripts"
    udf_dir = gdir / "Data" / "NVSE" / "user_defined_functions"

    if scripts_dir.is_dir():
        for item in scripts_dir.iterdir():
            if item.is_file() and item.name.startswith(STAGED_GR_PREFIX):
                try:
                    item.unlink()
                except OSError:
                    pass

    if udf_dir.is_dir():
        for item in udf_dir.iterdir():
            if item.is_file() and (
                item.name.startswith(STAGED_STEP_PREFIX) or item.name == STAGED_LOOP_FILENAME
            ):
                try:
                    item.unlink()
                except OSError:
                    pass


def stage_campaign_scripts(
    game_dir: str | os.PathLike[str],
    campaign: fos_campaigns.Campaign,
    *,
    interval: int | None = None,
) -> tuple[Path, Path]:
    """Write per-step `fosprobe_s_*.gek` UDFs, main loop UDF, and startup runner script."""
    gdir = Path(game_dir)
    scripts_dir = gdir / "Data" / "NVSE" / "Plugins" / "Scripts"
    udf_dir = gdir / "Data" / "NVSE" / "user_defined_functions"
    scripts_dir.mkdir(parents=True, exist_ok=True)
    udf_dir.mkdir(parents=True, exist_ok=True)

    cleanup_staged_scripts(gdir)

    for idx, step in enumerate(campaign.steps, 1):
        sp = udf_dir / f"{STAGED_STEP_PREFIX}{idx:02d}_{step.name}.gek"
        lines = ["begin Function {}"] + [f"  {cmd}" for cmd in step.commands] + ["end"]
        sp.write_bytes(("\r\n".join(lines) + "\r\n").encode("ascii"))

    tick_interval = campaign.interval if interval is None else interval
    gek = [
        "begin Function {}",
        '  int st = Player.AuxVarGetFlt "*fp_st"',
        "  int mm = GetActiveMenuMode",
        '  Player.AuxVarSetFlt "*fp_st" (st + 1)',
        "  if mm == 1013",
        "    if st == 4",
        '      WriteStringToFile "fosprobe_trace.txt" 1 "COC_START%r"',
        "      CenterOnCell GSDocMitchellHouse",
        "    endif",
        "    return",
        "  elseif mm != 0",
        "    CloseActiveMenu",
        '    ClickMenuButton "MessageMenu#1"',
        "    return",
        "  endif",
        '  int wt = Player.AuxVarGetFlt "*fp_wt"',
        "  if wt > 0",
        '    Player.AuxVarSetFlt "*fp_wt" (wt - 1)',
        "    return",
        "  endif",
        '  int ph = Player.AuxVarGetFlt "*fp_ph"',
        '  int sub = Player.AuxVarGetFlt "*fp_sub"',
        "  ref rFn",
        "  if ph == 0",
        '    Player.AuxVarSetFlt "*fp_ph" 1',
        '    Player.AuxVarSetFlt "*fp_wt" 8',
        "    return",
    ]
    for idx, step in enumerate(campaign.steps, 1):
        step_filename = f"{STAGED_STEP_PREFIX}{idx:02d}_{step.name}.gek"
        gek.append(f"  elseif ph == {idx}")
        gek.append("    if sub == 0")
        gek.append(f'      let rFn := CompileScript "{step_filename}"')
        gek.append("      if rFn")
        gek.append(f'        WriteStringToFile "fosprobe_trace.txt" 1 "RUN {idx} {step.name} OK%r"')
        gek.append("        Call rFn")
        gek.append("      else")
        gek.append(
            f'        WriteStringToFile "fosprobe_trace.txt" 1 "RUN {idx} {step.name} COMPILE_FAIL%r"'
        )
        gek.append("      endif")
        gek.append('      Player.AuxVarSetFlt "*fp_sub" 1')
        if step.wait_ticks > 0:
            gek.append(f'      Player.AuxVarSetFlt "*fp_wt" {step.wait_ticks}')
        gek.append("    else")
        gek.append(f'      WriteStringToFile "fosprobe_trace.txt" 1 "SAVE {idx} {step.name}%r"')
        gek.append(f"      con_Save {step.name}")
        gek.append('      Player.AuxVarSetFlt "*fp_sub" 0')
        gek.append(f'      Player.AuxVarSetFlt "*fp_ph" {idx + 1}')
        gek.append('      Player.AuxVarSetFlt "*fp_wt" 1')
        gek.append("    endif")

    final_ph = len(campaign.steps) + 1
    gek.extend(
        [
            f"  elseif ph == {final_ph}",
            '    WriteStringToFile "fosprobe_trace.txt" 1 "CAMPAIGN_DONE_QUIT%r"',
            f'    Player.AuxVarSetFlt "*fp_ph" {final_ph + 1}',
            "    con_QuitGame",
            "  endif",
            "end",
        ]
    )

    loop_path = udf_dir / STAGED_LOOP_FILENAME
    loop_path.write_bytes(("\r\n".join(gek) + "\r\n").encode("ascii"))

    gr_path = scripts_dir / f"{STAGED_GR_PREFIX}.txt"
    gr_lines = [
        f'WriteStringToFile "fosprobe_trace.txt" 1 "GR_START {campaign.name}%r"',
        'Player.AuxVarSetFlt "*fp_st" 0',
        'Player.AuxVarSetFlt "*fp_ph" 0',
        'Player.AuxVarSetFlt "*fp_sub" 0',
        'Player.AuxVarSetFlt "*fp_wt" 0',
        f'SetGameMainLoopCallback (CompileScript "{STAGED_LOOP_FILENAME}") 1 {tick_interval} 3',
        'WriteStringToFile "fosprobe_trace.txt" 1 "GR_DONE%r"',
    ]
    gr_path.write_bytes(("\r\n".join(gr_lines) + "\r\n").encode("ascii"))
    return loop_path, gr_path


def _clean_probe_save_dir(probe_save_dir: Path) -> None:
    if not probe_save_dir.is_dir():
        return
    for item in probe_save_dir.iterdir():
        if item.is_file():
            try:
                item.unlink()
            except OSError:
                pass


def run_campaign(
    *,
    campaign: fos_campaigns.Campaign,
    game_dir: str | os.PathLike[str],
    ini_dir: str | os.PathLike[str],
    out_dir: str | os.PathLike[str],
    loadorder_dir: str | os.PathLike[str] | None = None,
    enable_synthetic_esp: bool = False,
    width: int = 1280,
    height: int = 720,
    d3d_device: str | None = None,
    interval: int | None = None,
    max_wait: int | None = None,
    wait_fnv_free_seconds: float = 0.0,
    dry_run: bool = False,
    keep_staged: bool = False,
    is_running_fn: Callable[[], bool] = default_is_fnv_running,
) -> list[Path]:
    """Execute one declarative A/B `.fos` capture campaign safely."""
    if campaign.requires_synthetic_esp and not enable_synthetic_esp:
        raise ValueError(
            f"Campaign '{campaign.name}' requires --enable-synthetic-esp to build and "
            f"temporarily register {SYNTHETIC_ESP_FILENAME}."
        )

    gdir = Path(game_dir)
    idir = Path(ini_dir)
    odir = Path(out_dir)
    probe_save_dir = idir / PROBE_SAVE_SUBDIR
    fallout_ini = idir / "Fallout.ini"
    prefs_ini = idir / "FalloutPrefs.ini"
    custom_ini = idir / "FalloutCustom.ini"
    trace_path = gdir / "fosprobe_trace.txt"
    esp_dst = gdir / "Data" / SYNTHETIC_ESP_FILENAME

    if not dry_run:
        check_no_running_fnv(is_running=is_running_fn, wait_seconds=wait_fnv_free_seconds)

    if not fallout_ini.is_file() or not prefs_ini.is_file():
        raise FileNotFoundError(
            f"Expected Fallout.ini and FalloutPrefs.ini inside --ini-dir ({idir})"
        )

    protected: list[Path] = [fallout_ini, prefs_ini, custom_ini]
    plugins_txt: Path | None = None
    loadorder_txt: Path | None = None
    if loadorder_dir is not None:
        ldir = Path(loadorder_dir)
        plugins_txt = ldir / "plugins.txt"
        loadorder_txt = ldir / "loadorder.txt"
        protected.extend([plugins_txt, loadorder_txt])
    elif enable_synthetic_esp:
        raise ValueError(
            "--loadorder-dir (or FNV_LOADORDER_DIR) is required with --enable-synthetic-esp "
            "so plugins.txt and loadorder.txt can be backed up and restored."
        )

    probe_save_dir.mkdir(parents=True, exist_ok=True)
    odir.mkdir(parents=True, exist_ok=True)
    _clean_probe_save_dir(probe_save_dir)
    if trace_path.exists():
        trace_path.unlink()

    copied: list[Path] = []
    proc: subprocess.Popen[bytes] | None = None

    with FileBackupGuard(protected):
        try:
            if enable_synthetic_esp:
                esm_path = gdir / "Data" / "FalloutNV.esm"
                build_synthetic_probe_esp(esm_path, esp_dst)
                for lp in (plugins_txt, loadorder_txt):
                    if lp is not None and lp.is_file():
                        raw = lp.read_bytes()
                        if SYNTHETIC_ESP_FILENAME.encode("ascii") not in raw:
                            sep = b"" if raw.endswith(b"\n") else b"\r\n"
                            lp.write_bytes(raw + sep + SYNTHETIC_ESP_FILENAME.encode("ascii") + b"\r\n")

            ini_mod = patch_ini(
                fallout_ini.read_bytes().decode("latin-1"),
                "General",
                {
                    "SStartingCell": "",
                    "SCharGenQuest": campaign.start_chargen_quest,
                    "SLocalSavePath": "Saves\\fosprobe\\",
                    "bAllowConsole": "1",
                    "SIntroSequence": "",
                    "SMainMenuMovieIntro": "",
                    "sIntroMovie": "",
                },
            )
            display_updates = {
                "bFull Screen": "0",
                "iSize W": str(width),
                "iSize H": str(height),
            }
            if d3d_device:
                display_updates["sD3DDevice"] = d3d_device
            prefs_mod = patch_ini(
                prefs_ini.read_bytes().decode("latin-1"),
                "Display",
                display_updates,
            )
            custom_mod = (
                "[General]\r\n"
                f"SCharGenQuest={campaign.start_chargen_quest}\r\n"
                "sIntroMovie=\r\n"
                "SLocalSavePath=Saves\\fosprobe\\\r\n"
                "[Display]\r\n"
                "bFull Screen=0\r\n"
                f"iSize W={width}\r\n"
                f"iSize H={height}\r\n"
            )
            fallout_ini.write_bytes(ini_mod.encode("latin-1"))
            prefs_ini.write_bytes(prefs_mod.encode("latin-1"))
            custom_ini.write_bytes(custom_mod.encode("latin-1"))

            stage_campaign_scripts(gdir, campaign, interval=interval)
            if dry_run:
                return []

            exe_path = gdir / "FalloutNV.exe"
            if not exe_path.is_file():
                raise FileNotFoundError(f"FalloutNV.exe not found in --game-dir ({gdir})")

            timeout = campaign.max_wait if max_wait is None else max_wait
            print(f"=== Starting campaign '{campaign.name}' ({len(campaign.steps)} saves) ===")
            t0 = time.monotonic()
            proc = subprocess.Popen([str(exe_path)], cwd=str(gdir))
            last_count = -1
            while time.monotonic() - t0 < timeout:
                time.sleep(2.0)
                rc = proc.poll()
                fos_files = sorted(fn for fn in os.listdir(probe_save_dir) if fn.endswith(".fos"))
                if len(fos_files) != last_count or rc is not None:
                    latest = fos_files[-1] if fos_files else None
                    print(
                        f"  t={time.monotonic()-t0:.1f}s rc={rc} "
                        f"saves={len(fos_files)}/{len(campaign.steps)} latest={latest}"
                    )
                    last_count = len(fos_files)
                if rc is not None:
                    break
                if len(fos_files) >= len(campaign.steps):
                    time.sleep(2.5)
                    break

            if proc.poll() is None:
                terminate_pid_tree(proc.pid)
            time.sleep(0.5)

            for fn in sorted(os.listdir(probe_save_dir)):
                if fn.endswith(".fos"):
                    src = probe_save_dir / fn
                    dst = odir / fn
                    shutil.copy2(src, dst)
                    copied.append(dst)

            print(
                f"=== Campaign '{campaign.name}' copied "
                f"{len(copied)}/{len(campaign.steps)} .fos files ==="
            )
        finally:
            if proc is not None and proc.poll() is None:
                terminate_pid_tree(proc.pid)
            if not keep_staged:
                cleanup_staged_scripts(gdir)
            if enable_synthetic_esp and esp_dst.exists():
                try:
                    esp_dst.unlink()
                except OSError:
                    pass
            _clean_probe_save_dir(probe_save_dir)

    return copied


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(
        description="Clean-room parameterized black-box `.fos` A/B save-pair capture harness."
    )
    ap.add_argument(
        "--campaign",
        required=True,
        help="Campaign name to run, or 'list' to list all declarative campaigns.",
    )
    ap.add_argument(
        "--game-dir",
        default=os.environ.get("FNV_GAME_DIR"),
        help="Directory containing FalloutNV.exe and Data/ (or env FNV_GAME_DIR).",
    )
    ap.add_argument(
        "--ini-dir",
        default=os.environ.get("FNV_INI_DIR"),
        help="Directory containing Fallout.ini, FalloutPrefs.ini, Saves/ (or env FNV_INI_DIR).",
    )
    ap.add_argument(
        "--loadorder-dir",
        default=os.environ.get("FNV_LOADORDER_DIR"),
        help="Directory containing plugins.txt and loadorder.txt (or env FNV_LOADORDER_DIR).",
    )
    ap.add_argument(
        "--out-dir",
        default=os.environ.get("FNV_FOS_OUT_DIR"),
        help="Private output directory outside git for captured .fos files (or env FNV_FOS_OUT_DIR).",
    )
    ap.add_argument(
        "--enable-synthetic-esp",
        action="store_true",
        help="Build and temporarily register clean-room fos_probe_r3.esp for Group B campaigns.",
    )
    ap.add_argument("--width", type=int, default=1280, help="Window width (default 1280).")
    ap.add_argument("--height", type=int, default=720, help="Window height (default 720).")
    ap.add_argument("--d3d-device", default=None, help="Optional sD3DDevice override string.")
    ap.add_argument("--interval", type=int, default=None, help="Override script loop tick interval.")
    ap.add_argument("--max-wait", type=int, default=None, help="Override max wait seconds.")
    ap.add_argument(
        "--wait-fnv-free",
        type=float,
        default=0.0,
        help="Seconds to wait for an existing FalloutNV.exe to exit before failing (default 0).",
    )
    ap.add_argument(
        "--dry-run",
        action="store_true",
        help="Stage scripts and verify INI backup/restore without launching FalloutNV.exe.",
    )
    ap.add_argument(
        "--keep-staged",
        action="store_true",
        help="Keep staged fosprobe_* / gr_fosprobe.txt scripts after exit (default: remove in finally).",
    )

    args = ap.parse_args(argv)
    if args.campaign == "list":
        for camp in fos_campaigns.ALL_CAMPAIGNS:
            esp_tag = " [synthetic-esp]" if camp.requires_synthetic_esp else ""
            print(
                f"  R{camp.round_num} {camp.name:28s} ({len(camp.steps):2d} steps){esp_tag} — "
                f"{camp.description}"
            )
        return 0

    if args.campaign not in fos_campaigns.CAMPAIGNS_BY_NAME:
        ap.error(f"Unknown campaign {args.campaign!r}; pass --campaign list to see available names.")

    missing = [
        flag
        for flag, val in (
            ("--game-dir", args.game_dir),
            ("--ini-dir", args.ini_dir),
            ("--out-dir", args.out_dir),
        )
        if not val
    ]
    if missing:
        ap.error(f"Missing required path arguments (or env vars): {', '.join(missing)}")

    run_campaign(
        campaign=fos_campaigns.CAMPAIGNS_BY_NAME[args.campaign],
        game_dir=args.game_dir,
        ini_dir=args.ini_dir,
        out_dir=args.out_dir,
        loadorder_dir=args.loadorder_dir,
        enable_synthetic_esp=args.enable_synthetic_esp,
        width=args.width,
        height=args.height,
        d3d_device=args.d3d_device,
        interval=args.interval,
        max_wait=args.max_wait,
        wait_fnv_free_seconds=args.wait_fnv_free,
        dry_run=args.dry_run,
        keep_staged=args.keep_staged,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
