#!/usr/bin/env python3
"""Pure-synthetic offline unit test suite for `tools/capture/fos/`.

Constructs minimal `FO3SAVEGAME` buffers and minimal TES4/FNV ESMs entirely in
memory or `tempfile.TemporaryDirectory()`. Requires ZERO game files, ZERO `.fos`
fixtures, ZERO `.esm`/`.esp` assets, and ZERO third-party dependencies.
"""

from __future__ import annotations

import io
import os
import re
import stat
import struct
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

FOS_DIR = Path(__file__).resolve().parent.parent
if str(FOS_DIR) not in sys.path:
    sys.path.insert(0, str(FOS_DIR))

import campaigns as fos_campaigns  # noqa: E402
import harness as fos_harness  # noqa: E402
import parser as fos_parser  # noqa: E402
import verify as fos_verify  # noqa: E402

BAR = b"|"


def _bar_u8(v: int) -> bytes:
    return struct.pack("<B", v) + BAR


def _bar_i8(v: int) -> bytes:
    return struct.pack("<b", v) + BAR


def _bar_u16(v: int) -> bytes:
    return struct.pack("<H", v) + BAR


def _bar_i16(v: int) -> bytes:
    return struct.pack("<h", v) + BAR


def _bar_u32(v: int) -> bytes:
    return struct.pack("<I", v) + BAR


def _bar_i32(v: int) -> bytes:
    return struct.pack("<i", v) + BAR


def _bar_f32(v: float) -> bytes:
    return struct.pack("<f", v) + BAR


def _bar_f64(v: float) -> bytes:
    return struct.pack("<d", v) + BAR


def _bar_vsval(v: int) -> bytes:
    return fos_parser.encode_vsval(v) + BAR


def _bar_str(text: str) -> bytes:
    raw = text.encode("latin-1")
    if not raw:
        return _bar_u16(0)
    return _bar_u16(len(raw)) + raw + BAR


class SyntheticFosBuilder:
    """Minimal in-memory `FO3SAVEGAME` binary builder for offline testing."""

    def __init__(
        self,
        *,
        plugins: list[str] | None = None,
        player_name: str = "Courier",
        shot_w: int = 2,
        shot_h: int = 2,
        shot_rgb: tuple[int, int, int] = (120, 90, 50),
    ) -> None:
        self.plugins = plugins if plugins is not None else ["FalloutNV.esm"]
        self.player_name = player_name
        self.shot_w = shot_w
        self.shot_h = shot_h
        self.shot_rgb = shot_rgb
        self.form_ids: list[int] = []
        self.worldspaces: list[int] = [0x000DA726]
        self.globals1: dict[int, bytes] = {}
        self.globals2: dict[int, bytes] = {1000: b""}
        self.changed: list[tuple[bytes, int, int, int, bytes]] = []

    def ref3(self, fid: int | None) -> bytes:
        if fid is None:
            return b"\x00\x00\x00"
        if (fid >> 24) == 0xFF:
            return fos_parser.encode_ref3(2, fid & 0x3FFFFF)
        if fid not in self.form_ids:
            self.form_ids.append(fid)
        idx = self.form_ids.index(fid) + 1
        return fos_parser.encode_ref3(0, idx)

    def bar_ref3(self, fid: int | None) -> bytes:
        return self.ref3(fid) + BAR

    def add_changed(
        self, kind: int, fid: int, flags: int, data: bytes, *, ver: int = 15
    ) -> None:
        self.changed.append((self.ref3(fid), flags, kind, ver, data))

    def build(self) -> bytes:
        hdr = bytearray()
        hdr += _bar_u32(0x86)
        hdr += b"English".ljust(64, b"\x00") + BAR
        hdr += _bar_u32(self.shot_w)
        hdr += _bar_u32(self.shot_h)
        hdr += _bar_u32(1)
        hdr += _bar_str(self.player_name)
        hdr += _bar_str("Drifter")
        hdr += _bar_u32(1)
        hdr += _bar_str("Goodsprings")
        hdr += _bar_str("000.01.00")

        shot = bytes(self.shot_rgb) * (self.shot_w * self.shot_h)

        pb = bytearray()
        pb += _bar_u8(len(self.plugins))
        for p in self.plugins:
            pb += _bar_str(p)

        g1_blob = bytearray()
        for gtype, gdata in sorted(self.globals1.items()):
            g1_blob += struct.pack("<II", gtype, len(gdata)) + gdata

        ch_blob = bytearray()
        for r3, flags, kind, ver, data in self.changed:
            dlen = len(data)
            if dlen <= 0xFF:
                lw, len_bytes = 0, struct.pack("<B", dlen)
            elif dlen <= 0xFFFF:
                lw, len_bytes = 1, struct.pack("<H", dlen)
            else:
                lw, len_bytes = 2, struct.pack("<I", dlen)
            tb = (lw << 6) | (kind & 0x3F)
            ch_blob += r3 + struct.pack("<IBB", flags, tb, ver) + len_bytes + data

        g2_blob = bytearray()
        for gtype, gdata in sorted(self.globals2.items()):
            g2_blob += struct.pack("<II", gtype, len(gdata)) + gdata

        fid_blob = bytearray()
        fid_blob += struct.pack("<I", len(self.form_ids))
        for fid in self.form_ids:
            fid_blob += struct.pack("<I", fid)
        fid_blob += struct.pack("<I", len(self.worldspaces))
        for ws in self.worldspaces:
            fid_blob += struct.pack("<I", ws)

        prefix_len = (
            11 + 4 + len(hdr) + len(shot) + 1 + 4 + len(pb) + 32
        )
        g1_off = prefix_len
        changed_off = g1_off + len(g1_blob)
        g2_off = changed_off + len(ch_blob)
        formid_off = g2_off + len(g2_blob)
        tail_off = formid_off + len(fid_blob)

        table = struct.pack(
            "<8I",
            formid_off,
            tail_off,
            g1_off,
            changed_off,
            g2_off,
            len(self.globals1),
            len(self.globals2),
            len(self.changed),
        )

        return (
            b"FO3SAVEGAME"
            + struct.pack("<I", len(hdr))
            + bytes(hdr)
            + shot
            + struct.pack("<BI", 15, len(pb))
            + bytes(pb)
            + table
            + bytes(g1_blob)
            + bytes(ch_blob)
            + bytes(g2_blob)
            + bytes(fid_blob)
            + b"\x00" * 16
        )


def _build_player_payload(
    b: SyntheticFosBuilder,
    *,
    flags: int,
    av_overrides: dict[int, float] | None = None,
    scale: float | None = None,
    extras: bytes = b"",
    extra_count: int = 0,
    inventory: bytes | None = None,
    tok76_rounds: int = 13,
    bit22_entries: list[tuple[int, float]] | None = None,
    tail_notes: list[int] | None = None,
    perks: list[tuple[int, int]] | None = None,
    teammate_perks: list[tuple[int, int]] | None = None,
    active_effects: list[dict] | None = None,
) -> bytes:
    buf = bytearray()
    # Placement: cell ref3 + 6 f32 + |
    buf += b.ref3(0x00103DF9) + struct.pack("<6f", 1.0, 2.0, 3.0, 0.0, 0.0, 0.0) + BAR
    avs = [0.0] * fos_parser.AV_ENTRIES
    if av_overrides:
        for idx, val in av_overrides.items():
            avs[idx] = val
    for val in avs:
        buf += _bar_f32(val)
    buf += _bar_u8(0)  # process_level
    if scale is not None:
        buf += _bar_f32(scale)
    buf += _bar_vsval(extra_count) + extras
    buf += inventory if inventory is not None else _bar_vsval(0)

    # Post-inventory tail: 76 dummy tokens + tok[76] magazine rounds
    tail = bytearray()
    for _ in range(76):
        tail += _bar_u8(0)
    tail += _bar_u32(tok76_rounds)

    # Optional bit-22 AV overrides immediately before PROCESS_AV_LIST_ANCHOR
    if bit22_entries:
        tail += _bar_vsval(len(bit22_entries))
        for code, val in bit22_entries:
            tail += _bar_u8(code) + _bar_f32(val)
    tail += fos_parser.PROCESS_AV_LIST_ANCHOR

    # Tail notes list (BUG #9): 10 * (00 00 00 |) + vsval(0)| + vsval(n)| + n*(ref3|)
    tail += b"\x00\x00\x00\x7c" * 10
    tail += _bar_vsval(0)
    notes = tail_notes or []
    tail += _bar_vsval(len(notes))
    for nfid in notes:
        tail += b.bar_ref3(nfid)

    # Dead signature + perks + 4*(0|) + 5*(0u32|)
    tail += b"\xad\xde\xad\xde\x7c"
    tail += b"\x00\x7c\x00\x7c\x00\x7c"  # preceding bars required by perk matcher
    if perks:
        tail += _bar_vsval(len(perks))
        for pfid, rank in perks:
            tail += b.bar_ref3(pfid) + _bar_u8(rank)
    tail += b"\x00\x7c\x00\x7c\x00\x7c\x00\x7c"
    tail += b"\x00\x00\x00\x00\x7c" * 5

    # Active effects immediately before the 15-byte pre-teammate-perk prefix
    if active_effects:
        tail += _bar_vsval(len(active_effects))
        for eff in active_effects:
            pl = bytearray()
            pl += _bar_f32(eff["elapsed"])
            for u in eff.get("u8s", [0, 0, 0, 0, 0]):
                pl += _bar_u8(u)
            pl += _bar_f32(eff["magnitude"])
            pl += _bar_f32(eff["duration"])
            pl += _bar_u32(eff.get("flags", 0))
            pl += b.bar_ref3(eff.get("caster", 0x14))
            pl += b.bar_ref3(eff.get("target", 0x14))
            pl += b.bar_ref3(eff.get("extra_ref"))
            pl += b.bar_ref3(eff.get("worn"))
            tail += b.bar_ref3(eff["source"])
            tail += _bar_u8(eff.get("effect_index", 0))
            tail += _bar_vsval(eff.get("subtype", 0))
            tail += _bar_vsval(len(pl))
            tail += pl

    # 15-byte separator before teammate perks (`u32| u32| u32|`)
    tail += _bar_u32(0) + _bar_u32(0) + _bar_u32(0)
    if teammate_perks:
        tail += _bar_vsval(len(teammate_perks))
        for pfid, rank in teammate_perks:
            tail += b.bar_ref3(pfid) + _bar_u8(rank)
    else:
        tail += _bar_u8(0)

    # 54-byte end trailer: 00|01|01| + 8*(u32|) + 4*(u8|)
    tail += b"\x00\x7c\x01\x7c\x01\x7c"
    tail += b"\x00\x00\x00\x00\x7c" * 8
    tail += b"\x00\x7c" * 4

    return bytes(buf + tail)


class TestCursorAndPrimitives(unittest.TestCase):
    def test_vsval_roundtrip_and_invalid_tag(self) -> None:
        for v in (0, 1, 63, 64, 16383, 16384, 100000):
            raw = fos_parser.encode_vsval(v)
            c = fos_parser.Cursor(raw)
            self.assertEqual(c.vsval(), v)
            self.assertTrue(c.at_end())
        with self.assertRaises(fos_parser.ParseError):
            fos_parser.Cursor(b"\x03").vsval()

    def test_cursor_errors(self) -> None:
        with self.assertRaises(fos_parser.ParseError):
            fos_parser.Cursor(b"\x01").bar()
        with self.assertRaises(fos_parser.ParseError):
            fos_parser.Cursor(b"\x00").u32()
        with self.assertRaises(fos_parser.ParseError):
            fos_parser.FosFile.from_bytes(b"NOTAFOSFILE0000")

    def test_ref3_resolution_all_kinds(self) -> None:
        b = SyntheticFosBuilder()
        r0 = b.ref3(0x00104C0C)
        fos = fos_parser.FosFile.from_bytes(b.build())
        self.assertIsNone(fos.resolve(b"\x00\x00\x00"))
        self.assertEqual(fos.resolve(r0), 0x00104C0C)
        self.assertEqual(fos.resolve(fos_parser.encode_ref3(1, 0x14)), 0x14)
        self.assertEqual(fos.resolve(fos_parser.encode_ref3(2, 0x131C)), 0xFF00131C)


class TestGlobalsAndChangedForms(unittest.TestCase):
    def test_globals_0_1_2_8_10_and_bug3_radio(self) -> None:
        b = SyntheticFosBuilder(shot_rgb=(126, 111, 83))
        # Global[0]: misc stats (43 entries)
        stats = [0] * 43
        stats[1] = 1
        stats[7] = 2
        b.globals1[0] = _bar_u32(43) + b"".join(_bar_u32(x) for x in stats)
        # Global[1]: active quest
        b.globals1[1] = (
            _bar_u32(0x1310)
            + b.bar_ref3(0x000DA726)
            + _bar_i32(-18)
            + _bar_i32(0)
            + b.bar_ref3(0x000DA726)
            + struct.pack("<3f", 1.0, 2.0, 3.0)
            + BAR
            + b.bar_ref3(0x0010A214)
        )
        # Global[2]: dead counts
        b.globals1[2] = _bar_vsval(1) + b.bar_ref3(0x00104C0C) + _bar_u16(1) + (b"\x00" * 105)
        # Global[8]: weather
        b.globals1[8] = b.bar_ref3(0x000FFC88) + b.bar_ref3(None) + b"\x00" * 10
        # Global[10]: BUG #3 — opening on=0, active station in second vsval list
        b.globals1[10] = (
            _bar_u32(0)
            + _bar_u8(0)
            + _bar_str("")
            + _bar_vsval(0)
            + _bar_vsval(1)
            + b.bar_ref3(0x0014DF03)
            + _bar_u32(9740)
            + _bar_u32(1)
            + _bar_u8(100)
            + _bar_u8(100)
            + _bar_u32(51)
            + _bar_u8(0)
            + _bar_u8(0)
        )

        fos = fos_parser.FosFile.from_bytes(b.build())
        self.assertEqual(fos.screenshot_rgb_means(), (126.0, 111.0, 83.0))
        self.assertEqual(fos.decode_misc_stats()[7], 2)
        self.assertEqual(fos.decode_active_quest(), 0x0010A214)
        self.assertEqual(fos.decode_dead_counts()["entries"], [(0x00104C0C, 1)])
        self.assertEqual(fos.decode_weather()["current"], 0x000FFC88)
        rad = fos.decode_radio()
        self.assertTrue(rad["ok"])
        self.assertEqual(rad["on"], 0)
        self.assertTrue(rad["pipboy_on"])
        self.assertEqual(rad["pipboy_station"], 0x0014DF03)
        self.assertEqual(rad["active_stations"][0]["playback_ms"], 9740)
        self.assertEqual(fos.globals2, {1000: b""})

    def test_player_bugs_1_2_7_8_9_12_13(self) -> None:
        # Save A: bit 4 UNSET (BUG #1), AV2 rads/HC (BUG #7), bit 22 Luck (BUG #2),
        # tail notes (BUG #9), 0x3E no-unequip (BUG #12), tok[76]=12 (BUG #13),
        # NPC_ 0x7 bit 24 SexChange (BUG #8) + multi-bit order + bit 11 FaceGen.
        b = SyntheticFosBuilder()
        # NPC_ 0x7 with bits 4 + 2 + 5 + 11 + 24
        npc_data = bytearray()
        npc_data += _bar_vsval(1) + b.bar_ref3(0x001768BC) + _bar_vsval(0)  # bit 4
        npc_data += bytes([7, 5, 5, 5, 5, 5, 5]) + BAR  # bit 2 SPECIAL
        npc_data += _bar_str("Orris's Thug")  # bit 5 name
        # bit 11 FaceGen (676B)
        npc_data += _bar_u8(1)
        for _ in range(50 + 30 + 50):
            npc_data += _bar_f32(0.0)
        npc_data += b.bar_ref3(0x0002DDEE)
        npc_data += b.bar_ref3(0x00004253)
        npc_data += _bar_f32(0.04)
        npc_data += _bar_u32(0x002E4654)
        npc_data += _bar_vsval(1) + b.bar_ref3(0x000B5BEA)
        npc_data += _bar_u8(1)  # bit 24 SexChange female
        b.add_changed(10, 7, (1 << 4) | (1 << 2) | (1 << 5) | (1 << 11) | (1 << 24), bytes(npc_data))

        # Actor extras: 0x5E factions + 0x1D teammates
        aex = (
            _bar_u8(0x5E)
            + _bar_vsval(1)
            + b.bar_ref3(0x00104C6E)
            + _bar_i8(2)
            + _bar_u8(0x1D)
            + _bar_vsval(1)
            + b.bar_ref3(0x00096BCE)
        )
        # Inventory item exercising 0x16, 0x3E (BUG #12), 0x25, 0x24, 0x21, 0x22,
        # 0x23, 0x1C, 0x30, 0x4A, 0x8D, 0x2F, 0x6E, plus defensive 0x17/0x26/0x28.
        item_extras = (
            _bar_u8(0x16)
            + _bar_u8(0x3E)
            + _bar_u8(0x25) + _bar_f32(75.0)
            + _bar_u8(0x24) + _bar_i16(2)
            + _bar_u8(0x21) + b.bar_ref3(0x00104C0C)
            + _bar_u8(0x22) + b.bar_ref3(0x00000038)
            + _bar_u8(0x23) + _bar_i32(1)
            + _bar_u8(0x1C) + b.bar_ref3(0x0A000810)
            + _bar_u8(0x30) + _bar_f32(1.5)
            + _bar_u8(0x4A) + _bar_u8(2)
            + _bar_u8(0x8D) + _bar_u8(3)
            + _bar_u8(0x2F) + _bar_u32(1) + _bar_u8(0)
            + _bar_u8(0x6E) + b.bar_ref3(0x0008ED03) + _bar_u32(7)
            + _bar_u8(0x17)
            + _bar_u8(0x26) + _bar_f32(10.0)
            + _bar_u8(0x28) + _bar_f32(50.0)
        )
        inv = _bar_vsval(1) + b.bar_ref3(0x000E3778) + _bar_i32(1) + _bar_vsval(1) + _bar_vsval(16) + item_extras

        achr_flags = 0x90000822 | (1 << 22)  # bit 4 intentionally OFF!
        payload_no_scale = _build_player_payload(
            b,
            flags=achr_flags,
            av_overrides={
                10: -1.0,       # AV1 AGI penalty
                77 + 23: 250.0, # AV2 karma
                77 + 24: 500.0, # AV2 XP
                77 + 54: 125.0, # AV2 rads (BUG #7)
                77 + 73: 150.0, # AV2 dehydration (BUG #7)
                154 + 25: -40.0, # AV3 limb dmg
                231: -40.0,     # AV[231] HP dmg
            },
            scale=None,
            extras=aex,
            extra_count=2,
            inventory=inv,
            tok76_rounds=12,
            bit22_entries=[(11, 9.0)],
            tail_notes=[0x00175AB0],
            perks=[(0x00031DD3, 1)],
            teammate_perks=[(0x00094EB9, 1)],
            active_effects=[
                {
                    "source": 0x00015163,
                    "effect_index": 0,
                    "elapsed": 4.125,
                    "magnitude": 2.0,
                    "duration": 240.0,
                }
            ],
        )
        b.add_changed(1, 0x14, achr_flags, payload_no_scale)
        fos = fos_parser.FosFile.from_bytes(b.build())

        # BUG #1: unconditional read fails when bit 4 is unset; gated read succeeds!
        with self.assertRaises(fos_parser.ParseError):
            fos.decode_player(gate_scale_bit4=False)

        # BUG #12: strict legacy mode rejects 0x3E!
        p_strict = fos.decode_player(strict_legacy_0x3e=True)
        self.assertIn("unknown item extra 0x3E", p_strict.get("inv_err", ""))

        # BUG #2: skill-only filter rejects Luck (11); general parser accepts it!
        p_skill_only = fos.decode_player(restrict_bit22_skills=True)
        self.assertIsNone(p_skill_only["bit22_av_overrides"])

        p = fos.decode_player()
        self.assertIsNone(p["scale"])
        self.assertEqual(p["av2_base"][54], 125.0)
        self.assertEqual(p["av2_base"][73], 150.0)
        self.assertEqual(p["av3_dmg"][25], -40.0)
        self.assertEqual(p["av231_hp_dmg"], -40.0)
        self.assertEqual(p["bit22_av_overrides"], [(11, 9.0)])
        self.assertEqual(p["tok76_mag_rounds"], 12)
        self.assertEqual(fos_verify.decode_player_tok76_from_file(fos), 12)
        self.assertEqual(p["tail_notes"], [0x00175AB0])
        self.assertTrue(fos_verify.has_achr_note_ref(fos, 0x00175AB0))
        self.assertEqual(p["perks"], [(0x00031DD3, 1)])
        self.assertEqual(p["teammate_perks"], [(0x00094EB9, 1)])
        self.assertEqual(len(p["active_effects"]), 1)
        self.assertAlmostEqual(p["active_effects"][0]["elapsed"], 4.125, places=3)

        npc = p["npc_base"]
        self.assertEqual(npc["rem"], 0)
        self.assertEqual(npc["special"], [7, 5, 5, 5, 5, 5, 5])
        self.assertEqual(npc["name"], "Orris's Thug")
        self.assertTrue(npc["female"])
        self.assertTrue(npc["sex_change_bit24"])
        self.assertEqual(npc["appearance"]["hair"], 0x0002DDEE)
        self.assertEqual(npc["appearance"]["eyes"], 0x00004253)
        self.assertEqual(npc["appearance"]["hcol"], 0x002E4654)
        self.assertEqual(npc["appearance"]["parts"], [0x000B5BEA])

        entry = p["inventory"][0]["entries"][0]
        self.assertTrue(entry["0x3E_cannot_wear_or_unequip"])
        self.assertEqual(entry["0x21_owner"], 0x00104C0C)
        self.assertEqual(entry["0x22"], 0x00000038)
        self.assertEqual(entry["0x23_rank"], 1)
        self.assertEqual(entry["0x1C"], 0x0A000810)
        self.assertEqual(entry["0x30"], 1.5)
        self.assertEqual(entry["0x4A_hotkey"], 2)
        self.assertEqual(entry["0x8D_mods"], 3)
        self.assertEqual(entry["0x6E_ammo"], (0x0008ED03, 7))

        # Now verify bit 4 SET with scale=1.25
        b2 = SyntheticFosBuilder()
        payload_scaled = _build_player_payload(b2, flags=0x90000032, scale=1.25)
        b2.add_changed(1, 0x14, 0x90000032, payload_scaled)
        fos2 = fos_parser.FosFile.from_bytes(b2.build())
        self.assertEqual(fos2.decode_player()["scale"], 1.25)

    def test_refr_bugs_4_10_11_13_and_other_records(self) -> None:
        b = SyntheticFosBuilder()

        # BUG #4: REFR bit 6 ownership + bit 4 scale + inventory + anim
        r_own = (
            _bar_f32(1.5)
            + _bar_vsval(1) + _bar_u8(0x21) + b.bar_ref3(0x00104C0C)
            + _bar_vsval(0)
            + _bar_vsval(8) + _bar_str("Close")
        )
        b.add_changed(0, 0x00103E4A, 0x18000050, r_own)

        # BUG #10: REFR destructible 0x56 HP + 0x5B stage 0
        r_dest = (
            _bar_u32(0x01840009)
            + _bar_vsval(2)
            + _bar_u8(0x56) + _bar_f32(0.0)
            + _bar_u8(0x5B) + b.bar_ref3(0x00020D70) + _bar_u32(0)
        )
        b.add_changed(0, 0x000EA490, 0x80000001, r_dest)

        # BUG #11: REFR bit 10 (0x400) with 0x1C holder
        r_bit10 = _bar_u32(0x0000042A) + _bar_vsval(1) + _bar_u8(0x1C) + b.bar_ref3(0x14)
        b.add_changed(0, 0x0A000810, 0x00000401, r_bit10)

        # BUG #13: Created 0xFF dropped WEAP REFR with 0x25 + 0x6E + 0x39
        r_drop = (
            b.ref3(0x00103DF9)
            + struct.pack("<6f", 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
            + struct.pack("<B", 1)
            + b.ref3(0x000E3778)
            + BAR
            + _bar_vsval(0)  # empty havok (bit 2)
            + _bar_f32(1.0)  # scale (bit 4)
            + _bar_vsval(3)
            + _bar_u8(0x25) + _bar_f32(149.85)
            + _bar_u8(0x6E) + b.bar_ref3(0x0008ED03) + _bar_u32(4)
            + _bar_u8(0x39) + b.bar_ref3(0xFF001312)
        )
        b.add_changed(0, 0xFF001316, 0x80000C16, r_drop)

        # BUG #5: CELL bit 1 + bit 31 fog + trailing bit 3 owner
        cell_data = _bar_u8(0x20) + (b"\x08\x7c" + b"\x00" * 20) + b.bar_ref3(0x00104C0C)
        b.add_changed(7, 0x00103DF9, 0x8000000A, cell_data)

        # BUG #6: Non-player ACHR with bit 23 ModAV + 0x5F limbs + 0x89 ash pile
        actor_extras = (
            _bar_u8(0x5F)
            + _bar_u16(2) + _bar_u32(0) + _bar_u32(0) + _bar_u8(0) + b.bar_ref3(None)
            + _bar_vsval(1) + _bar_u8(1) + _bar_u8(1) + _bar_u8(0) + _bar_u8(0) + _bar_vsval(0)
            + _bar_u8(0x89) + b.bar_ref3(0xFF001318)
        )
        actor_tail = _bar_vsval(1) + _bar_u8(5) + _bar_f32(4.0) + fos_parser.PROCESS_AV_LIST_ANCHOR
        actor_data = _bar_u8(0) + _bar_u32(0x00010409) + _bar_f32(1.35) + _bar_vsval(2) + actor_extras + actor_tail
        b.add_changed(1, 0x00104C0F, (1 << 0) | (1 << 4) | (1 << 23) | (1 << 31), actor_data)

        # PGRE, ECZN, TACT, MISC, QUST, FACT, FLST, LVLI, REPU, CHAL, CLAS, INFO
        b.add_changed(4, 0x000EA808, 0x1, _bar_u8(0xFF) + _bar_u32(0x0100080B))
        b.add_changed(32, 0x000D7034, (1 << 1) | (1 << 31), _bar_u8(1) + struct.pack("<4I", 0, 132, 0, 2) + BAR)
        b.add_changed(13, 0x0014DF02, (1 << 2) | (1 << 23), _bar_str("Orris's Thug") + b.bar_ref3(0x00104C0F))
        b.add_changed(22, 0x0005B63C, 0x3, _bar_u32(0x409) + _bar_i32(1234))
        b.add_changed(
            9,
            0x0010A214,
            0x20000006,
            _bar_u8(0x21) + _bar_f32(2.5) + _bar_vsval(1) + _bar_u32(10) + _bar_u32(3),
        )
        b.add_changed(
            34,
            0x00104C6E,
            0x6,
            _bar_u32(0x0111) + _bar_vsval(1) + b.bar_ref3(0x00104C7E) + _bar_i32(42) + _bar_u32(1),
        )
        b.add_changed(37, 0x0017BA3B, 0x80000000, _bar_u32(1) + b.bar_ref3(0x000E3778))
        b.add_changed(
            40,
            0x0016BACB,
            0x80000000,
            _bar_vsval(1) + b.bar_ref3(0x000E3778) + _bar_u16(3) + _bar_u16(2) + _bar_f32(0.0075),
        )
        b.add_changed(43, 0x00104C22, 0x2, _bar_f32(15.0) + _bar_f32(7.0))
        b.add_changed(50, 0x0015EAF1, 0x2, _bar_u32(1) + _bar_u32(0))
        b.add_changed(33, 0x00057E6A, 0x2, _bar_u32(0xFFFFFFFF) + _bar_u32(36) + _bar_u32(0xFFFFFFFF) + _bar_u32(0xFFFFFFFF))
        b.add_changed(8, 0x001751E9, 0x80000000, b"")

        fos = fos_parser.FosFile.from_bytes(b.build())

        # BUG #4 legacy vs fixed
        self.assertIsNotNone(fos.decode_refr(0x00103E4A, include_bit6_bit10=False).get("err"))
        r4 = fos.decode_refr(0x00103E4A)
        self.assertEqual(r4["scale"], 1.5)
        self.assertIn(("0x21_owner", 0x00104C0C), r4["extras"])
        self.assertEqual(r4["anim"]["name"], "Close")

        # BUG #10
        rd = fos.decode_refr(0x000EA490)
        self.assertIn(("0x56_hp", 0.0), rd["extras"])
        self.assertIn(("0x5B_dest", (0x00020D70, 0)), rd["extras"])

        # BUG #11 legacy vs fixed
        self.assertEqual(fos.decode_refr(0x0A000810, include_bit6_bit10=False)["rem"], 8)
        r11 = fos.decode_refr(0x0A000810)
        self.assertEqual(r11["rem"], 0)
        self.assertIn(("0x1C", 0x14), r11["extras"])

        # BUG #13
        r13 = fos.decode_refr(0xFF001316)
        self.assertIn(("0x6E_ammo", (0x0008ED03, 4)), r13["extras"])
        self.assertIn(("0x39_dropped_from", 0xFF001312), r13["extras"])

        # BUG #5
        cell = fos.decode_cell(0x00103DF9)
        self.assertEqual(cell["cell_flags"], 0x20)
        self.assertEqual(cell["owner"], 0x00104C0C)

        # BUG #6
        act = fos.decode_actor(1, 0x00104C0F)
        self.assertEqual(act["av_bit23_mod"], [(5, 4.0)])
        self.assertIn(("0x5F_limbs", (2, [(1, True)])), act["extras"])
        self.assertIn(("0x89_ashpile", 0xFF001318), act["extras"])

        # Other changed form decoders
        self.assertEqual(fos.decode_pgre(0x000EA808)["form_flags"], 0x0100080B)
        self.assertEqual(fos.decode_eczn(0x000D7034)["stamps"], (0, 132, 0, 2))
        tact = fos.decode_tact(0x0014DF02)
        self.assertEqual((tact["name"], tact["talking_actor"], tact["rem"]), ("Orris's Thug", 0x00104C0F, 0))
        misc = fos.decode_misc(0x0005B63C)
        self.assertEqual((misc["form_flags"], misc["value"]), (0x409, 1234))
        qust = fos.decode_qust(0x0010A214)
        self.assertEqual((qust["quest_flags"], qust["delay"], qust["objectives"]), (0x21, 2.5, [(10, 3)]))
        fact = fos.decode_fact(0x00104C6E)
        self.assertEqual(fact["faction_flags"], 0x0111)
        self.assertEqual(fact["relations"], [(0x00104C7E, 42, 1)])
        self.assertEqual(fos.decode_flst(0x0017BA3B)["forms"], [0x000E3778])
        self.assertEqual(fos.decode_lvli(0x0016BACB)["entries"][0][:3], (0x000E3778, 3, 2))
        self.assertEqual((fos.decode_repu(0x00104C22)["fame"], fos.decode_repu(0x00104C22)["infamy"]), (15.0, 7.0))
        self.assertEqual(fos.decode_chal(0x0015EAF1)["progress"], 1)
        self.assertEqual(fos.decode_clas(0x00057E6A)["tag_skills"][1], 36)

    def test_diff_saves(self) -> None:
        ba = SyntheticFosBuilder()
        ba.add_changed(22, 0x0005B63C, 0x2, _bar_i32(10))
        bb = SyntheticFosBuilder()
        bb.add_changed(22, 0x0005B63C, 0x2, _bar_i32(1234))
        bb.add_changed(43, 0x00104C22, 0x2, _bar_f32(15.0) + _bar_f32(7.0))
        bb.globals1[8] = bb.bar_ref3(0x000FFC88) + bb.bar_ref3(None)

        diff = fos_parser.diff_saves(
            fos_parser.FosFile.from_bytes(ba.build(), "a.fos"),
            fos_parser.FosFile.from_bytes(bb.build(), "b.fos"),
        )
        self.assertIn(8, diff["global_diffs"])
        self.assertEqual(len(diff["new_forms"]), 1)
        self.assertEqual(diff["new_forms"][0]["kind_name"], "REPU")
        self.assertEqual(len(diff["modified_forms"]), 1)
        self.assertEqual(diff["modified_forms"][0]["kind_name"], "MISC")
        text = fos_parser.format_diff(diff)
        self.assertIn("REPU", text)
        self.assertIn("MISC", text)


class TestCampaignsSchemaAndHygiene(unittest.TestCase):
    def test_campaign_schema_and_clean_room_hygiene(self) -> None:
        self.assertGreaterEqual(len(fos_campaigns.ALL_CAMPAIGNS), 18)
        self.assertEqual(len(fos_campaigns.ALL_CAMPAIGNS), len(fos_campaigns.CAMPAIGNS_BY_NAME))
        seen_steps: set[str] = set()
        for camp in fos_campaigns.ALL_CAMPAIGNS:
            self.assertIn(camp.round_num, (1, 2, 3))
            self.assertGreaterEqual(len(camp.steps), 2)
            for step in camp.steps:
                self.assertRegex(step.name, r"^[a-z0-9_]+$")
                self.assertNotIn(step.name, seen_steps, f"duplicate step name {step.name}")
                seen_steps.add(step.name)
                for cmd in step.commands:
                    cmd.encode("ascii")
                    self.assertNotRegex(cmd, r"[A-Za-z]:\\")

        self.assertEqual(fos_campaigns.COVERAGE_SUMMARY["total_converter_keys"], 87)
        self.assertEqual(fos_campaigns.COVERAGE_SUMMARY["unresolved_hypothesis_keys"], 0)
        self.assertEqual(fos_campaigns.COVERAGE_SUMMARY["verify_assertions"], 68)


class TestHarnessBackupEspAndStaging(unittest.TestCase):
    def test_patch_ini_updates_adds_and_preserves(self) -> None:
        orig = "[General]\r\n; keep comment\r\nSLocalSavePath=Saves\\\r\nKeepMe=1\r\n"
        patched = fos_harness.patch_ini(
            orig, "General", {"SLocalSavePath": "Saves\\fosprobe\\", "bAllowConsole": "1"}
        )
        self.assertIn("; keep comment", patched)
        self.assertIn("KeepMe=1", patched)
        self.assertIn("SLocalSavePath=Saves\\fosprobe\\", patched)
        self.assertIn("bAllowConsole=1", patched)

        patched2 = fos_harness.patch_ini(patched, "Display", {"bFull Screen": "0"})
        self.assertIn("[Display]\r\nbFull Screen=0\r\n", patched2)

    def test_file_backup_guard_restores_bytes_mode_and_removes_temp_files(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tdp = Path(td)
            ini = tdp / "Fallout.ini"
            prefs = tdp / "FalloutPrefs.ini"
            custom = tdp / "FalloutCustom.ini"
            orig_ini = b"[General]\r\nSLocalSavePath=Saves\\\r\n"
            orig_prefs = b"[Display]\r\nbFull Screen=1\r\n"
            ini.write_bytes(orig_ini)
            prefs.write_bytes(orig_prefs)
            os.chmod(ini, stat.S_IREAD)
            orig_ini_hash = fos_harness.sha256_file(ini)
            orig_prefs_hash = fos_harness.sha256_file(prefs)

            with self.assertRaises(RuntimeError):
                with fos_harness.FileBackupGuard([ini, prefs, custom]):
                    ini.write_bytes(b"MODIFIED")
                    prefs.write_bytes(b"MODIFIED")
                    custom.write_bytes(b"TEMP")
                    raise RuntimeError("simulated crash")

            self.assertEqual(ini.read_bytes(), orig_ini)
            self.assertEqual(fos_harness.sha256_file(ini), orig_ini_hash)
            self.assertEqual(prefs.read_bytes(), orig_prefs)
            self.assertEqual(fos_harness.sha256_file(prefs), orig_prefs_hash)
            self.assertFalse(custom.exists())
            # Make writable so TemporaryDirectory cleanup succeeds on Windows
            os.chmod(ini, stat.S_IWRITE | stat.S_IREAD)

    def test_collision_guard_refuses_when_running(self) -> None:
        fos_harness.check_no_running_fnv(is_running=lambda: False)
        with self.assertRaises(RuntimeError):
            fos_harness.check_no_running_fnv(is_running=lambda: True, wait_seconds=0.0)

    def test_synthetic_esp_builder_and_dry_run_end_to_end(self) -> None:
        # Build a tiny synthetic FalloutNV.esm with only the 7 template records.
        def mk_sub(sig: bytes, data: bytes) -> bytes:
            return fos_harness._subrec(sig, data)

        def mk_rec(sig: bytes, fid: int, subs: list[tuple[bytes, bytes]]) -> bytes:
            return fos_harness._record(sig, fid, b"".join(mk_sub(s, d) for s, d in subs))

        tes4 = mk_rec(b"TES4", 0, [(b"HEDR", struct.pack("<fII", 1.34, 7, 0x800))])
        ligh = mk_rec(b"LIGH", 0x00177ADD, [(b"EDID", b"TplLigh\x00")])
        armo = mk_rec(
            b"ARMO",
            0x00020423,
            [(b"EDID", b"TplArmo\x00"), (b"FULL", b"Leather\x00"), (b"BMDT", struct.pack("<IB3x", 4, 0))],
        )
        ench = mk_rec(
            b"ENCH",
            0x001673BF,
            [(b"EDID", b"TplEnch\x00"), (b"FULL", b"Ench\x00"), (b"ENIT", struct.pack("<IIII", 2, 0, 0, 0))],
        )
        weap1 = mk_rec(b"WEAP", 0x000E3778, [(b"EDID", b"Tpl9mm\x00"), (b"FULL", b"9mm\x00")])
        weap2 = mk_rec(b"WEAP", 0x00162C92, [(b"EDID", b"TplKnife\x00"), (b"FULL", b"Knife\x00")])
        cell = mk_rec(b"CELL", 0x00103DF9, [(b"EDID", b"GSDocMitchellHouse\x00")])
        refr = mk_rec(
            b"REFR",
            0x001059AE,
            [(b"EDID", b"TplPlunger\x00"), (b"DATA", struct.pack("<6f", 10.0, 20.0, 30.0, 0.0, 0.0, 0.0))],
        )
        esm_bytes = (
            tes4
            + fos_harness._grup(b"LIGH", 0, ligh)
            + fos_harness._grup(b"ARMO", 0, armo)
            + fos_harness._grup(b"ENCH", 0, ench)
            + fos_harness._grup(b"WEAP", 0, weap1 + weap2)
            + fos_harness._grup(b"CELL", 0, cell + refr)
        )

        with tempfile.TemporaryDirectory() as td:
            tdp = Path(td)
            gdir = tdp / "game"
            idir = tdp / "ini"
            ldir = tdp / "local"
            odir = tdp / "out"
            (gdir / "Data").mkdir(parents=True)
            idir.mkdir()
            ldir.mkdir()

            esm_path = gdir / "Data" / "FalloutNV.esm"
            esm_path.write_bytes(esm_bytes)
            (idir / "Fallout.ini").write_bytes(b"[General]\r\nSLocalSavePath=Saves\\\r\n")
            (idir / "FalloutPrefs.ini").write_bytes(b"[Display]\r\nbFull Screen=1\r\n")
            (ldir / "plugins.txt").write_bytes(b"FalloutNV.esm\r\n")
            (ldir / "loadorder.txt").write_bytes(b"FalloutNV.esm\r\n")

            # Standalone ESP builder check
            esp_test = tdp / "out.esp"
            fos_harness.build_synthetic_probe_esp(esm_path, esp_test)
            esp_raw = esp_test.read_bytes()
            self.assertTrue(esp_raw.startswith(b"TES4"))
            extracted = fos_harness._extract_esm_records(
                esp_test,
                {
                    (b"LIGH", 0x01000801),
                    (b"ARMO", 0x01000802),
                    (b"ENCH", 0x01000805),
                    (b"WEAP", 0x01000806),
                    (b"WEAP", 0x01000807),
                    (b"CELL", 0x00103DF9),
                    (b"REFR", 0x01000810),
                    (b"REFR", 0x01000815),
                },
            )
            self.assertEqual(len(extracted), 8)

            # Place unrelated user/mod scripts to prove harness never deletes them.
            udf_dir = gdir / "Data" / "NVSE" / "user_defined_functions"
            scripts_dir = gdir / "Data" / "NVSE" / "Plugins" / "Scripts"
            udf_dir.mkdir(parents=True, exist_ok=True)
            scripts_dir.mkdir(parents=True, exist_ok=True)
            user_udf = udf_dir / "s_user_mod.gek"
            user_udf.write_bytes(b"begin Function {}\r\nend\r\n")
            user_gr = scripts_dir / "gr_user_mod.txt"
            user_gr.write_bytes(b"; user startup script\r\n")

            # Direct staging check: creates fosprobe_s_* UDFs and preserves s_user_mod.gek
            loop_path, gr_path = fos_harness.stage_campaign_scripts(
                gdir, fos_campaigns.RUN21_SYNTHETIC_ESP
            )
            self.assertTrue(loop_path.is_file())
            self.assertTrue(gr_path.is_file())
            self.assertTrue((udf_dir / "fosprobe_s_01_r21_00_base.gek").is_file())
            self.assertIn(
                'CompileScript "fosprobe_s_01_r21_00_base.gek"',
                loop_path.read_text(encoding="ascii"),
            )
            self.assertTrue(user_udf.is_file())
            self.assertTrue(user_gr.is_file())

            # Dry-run campaign with --enable-synthetic-esp must clean ESP, remove all
            # harness-staged scripts in finally:, restore INIs & load order, and preserve
            # unrelated user/mod scripts (s_user_mod.gek, gr_user_mod.txt).
            fos_harness.run_campaign(
                campaign=fos_campaigns.RUN21_SYNTHETIC_ESP,
                game_dir=gdir,
                ini_dir=idir,
                out_dir=odir,
                loadorder_dir=ldir,
                enable_synthetic_esp=True,
                dry_run=True,
            )
            self.assertFalse((gdir / "Data" / fos_harness.SYNTHETIC_ESP_FILENAME).exists())
            self.assertEqual((ldir / "plugins.txt").read_bytes(), b"FalloutNV.esm\r\n")
            self.assertEqual((ldir / "loadorder.txt").read_bytes(), b"FalloutNV.esm\r\n")
            self.assertEqual(
                (idir / "Fallout.ini").read_bytes(), b"[General]\r\nSLocalSavePath=Saves\\\r\n"
            )
            self.assertFalse(loop_path.exists())
            self.assertFalse(gr_path.exists())
            self.assertEqual(
                [p.name for p in udf_dir.iterdir() if p.name.startswith("fosprobe_")], []
            )
            self.assertEqual(
                [p.name for p in scripts_dir.iterdir() if p.name.startswith("gr_fosprobe")], []
            )
            self.assertTrue(user_udf.is_file())
            self.assertEqual(user_udf.read_bytes(), b"begin Function {}\r\nend\r\n")
            self.assertTrue(user_gr.is_file())

            # Refuses synthetic-esp campaign without opt-in flag
            with self.assertRaises(ValueError):
                fos_harness.run_campaign(
                    campaign=fos_campaigns.RUN21_SYNTHETIC_ESP,
                    game_dir=gdir,
                    ini_dir=idir,
                    out_dir=odir,
                    enable_synthetic_esp=False,
                    dry_run=True,
                )

    def test_staged_scripts_cleaned_on_launch_error_and_user_udfs_preserved(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            tdp = Path(td)
            gdir = tdp / "game"
            idir = tdp / "ini"
            odir = tdp / "out"
            udf_dir = gdir / "Data" / "NVSE" / "user_defined_functions"
            scripts_dir = gdir / "Data" / "NVSE" / "Plugins" / "Scripts"
            udf_dir.mkdir(parents=True)
            scripts_dir.mkdir(parents=True)
            idir.mkdir()
            (idir / "Fallout.ini").write_bytes(b"[General]\r\nSLocalSavePath=Saves\\\r\n")
            (idir / "FalloutPrefs.ini").write_bytes(b"[Display]\r\nbFull Screen=1\r\n")
            user_udf = udf_dir / "s_user_mod.gek"
            user_udf.write_bytes(b"begin Function {}\r\nend\r\n")

            # Missing FalloutNV.exe raises FileNotFoundError AFTER staging; finally: must still clean up.
            with self.assertRaises(FileNotFoundError):
                fos_harness.run_campaign(
                    campaign=fos_campaigns.RUN1_PLAYER,
                    game_dir=gdir,
                    ini_dir=idir,
                    out_dir=odir,
                    dry_run=False,
                    is_running_fn=lambda: False,
                )

            self.assertFalse((udf_dir / "fosprobe_loop.gek").exists())
            self.assertFalse((scripts_dir / "gr_fosprobe.txt").exists())
            self.assertEqual([p.name for p in udf_dir.iterdir()], ["s_user_mod.gek"])
            self.assertEqual(list(scripts_dir.iterdir()), [])

    def test_cli_list_and_verify_missing_dir(self) -> None:
        buf = io.StringIO()
        with redirect_stdout(buf):
            rc = fos_harness.main(["--campaign", "list"])
        self.assertEqual(rc, 0)
        self.assertIn("run1_player", buf.getvalue())
        self.assertIn("run27_dead_npc_drop_0x6e", buf.getvalue())

        with self.assertRaises(FileNotFoundError):
            fos_verify.verify_directory(Path(tempfile.gettempdir()) / "nonexistent_fos_dir_xyz")


if __name__ == "__main__":
    unittest.main()
