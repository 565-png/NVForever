# Clean-Room `.fos` Save-Pair Capture, Parser & Verification Harness

This package provides a reproducible, black-box A/B capture runner, pure-Python
`.fos` binary parser/differ, and 68-assertion verification suite for validating
Fallout: New Vegas (`FO3SAVEGAME`) save layouts without ever committing game
assets, saves, plugins, or private machine paths.

## Clean-Room Charter

- **Black-box runtime observation only:** Every format check here operates on
  `.fos` save pairs produced by running unmodified *Fallout: New Vegas*
  `1.4.0.525` (English) and diffing Save A (before) vs Save B (after) a single
  state mutation. No disassembly, decompilation, binary string dumping, or
  third-party DLL inspection is used.
- **Zero game data in git:** Never commit `.fos` saves, `.esm`/`.esp` plugins,
  screenshots, or raw dumps. Always point `--out-dir` and `--saves-dir` at a
  private directory outside the repository.
- **Zero hardcoded paths or dependencies:** Standard library Python 3.11+ only.
  All directories (`--game-dir`, `--ini-dir`, `--loadorder-dir`, `--out-dir`,
  `--saves-dir`) come from CLI flags or environment variables (`FNV_GAME_DIR`,
  `FNV_INI_DIR`, `FNV_LOADORDER_DIR`, `FNV_FOS_OUT_DIR`, `FNV_FOS_SAVES_DIR`).

## Prerequisites (for live capture only)

- Offline synthetic tests (`tests/test_parser_synthetic.py`) require **no game
  install and no `.fos` files**.
- Live A/B capture (`harness.py`) requires your own install of *Fallout: New
  Vegas* `1.4.0.525` (English) with only the 10 official base masters loaded
  (`FalloutNV.esm`, the four story DLCs, `GunRunnersArsenal.esm`, and the four
  courier packs; zero gameplay/UI/tweak mods), plus an in-process script runner
  capable of compiling `.gek` UDFs and running `con_Save` / `con_QuitGame`.

## Layout

- `parser.py` — stdlib-only `.fos` binary parser (Header, Screenshot RGB mean
  check, Global Tables 1 & 2, FormID/worldspace tables, Changed Forms across
  `ACHR`/`ACRE`, `REFR`, `PGRE`, `CELL`, `QUST`, `NPC_`/`CREA`, `ECZN`, `TACT`,
  `MISC`, `FACT`, `FLST`, `LVLI`, `REPU`, `CHAL`, `CLAS`, `INFO`, and `0xFF`
  created refs) plus A/B save-pair diffing.
- `campaigns.py` — declarative single-variable A/B probe campaigns across
  Rounds 1–3 using `BuildRef 0 <id>` and vanilla commands.
- `harness.py` — parameterized runner with:
  - collision guard refusing to launch if `FalloutNV.exe` is already running,
  - PID-tree-only cleanup (never kills unrelated game instances),
  - SHA-256 + file-mode backup and `try/finally` verbatim restoration of
    `Fallout.ini`, `FalloutPrefs.ini`, optional `FalloutCustom.ini`,
    `plugins.txt`, and `loadorder.txt`,
  - isolated `<ini-dir>/Saves/fosprobe/` save folder cleanup `[design]`,
  - optional `--enable-synthetic-esp` on-demand TES4/FNV probe-plugin builder
    (removed in `try/finally`).
- `verify.py` — 68-assertion CLI verifier covering all confirmed `[observed]`
  layouts, 13 parser/converter regression triggers, and 3 `[observed]` negative
  proofs.
- `tests/test_parser_synthetic.py` — offline pure-synthetic unit test suite
  building minimal `FO3SAVEGAME` buffers and synthetic ESMs in memory/tempdirs.

## Usage

### 1. Run offline synthetic tests (no game required)

```bash
python -m unittest discover -s tools/capture/fos/tests -v
```

### 2. List or run live A/B capture campaigns (private output directory)

```bash
python tools/capture/fos/harness.py --campaign list

python tools/capture/fos/harness.py \
  --campaign run1_player \
  --game-dir "<path-to-fnv>" \
  --ini-dir "<path-to-My-Games-FalloutNV>" \
  --out-dir "<private-dir>/fos-saves"
```

Add `--dry-run` to stage scripts and verify INI backup/restore without starting
`FalloutNV.exe`.

### 3. Diff or scan captured `.fos` pairs

```bash
python tools/capture/fos/parser.py diff "<private-dir>/p00_base.fos" "<private-dir>/p01_scale.fos"
python tools/capture/fos/parser.py scan --saves-dir "<private-dir>/fos-saves"
```

### 4. Run the 68-assertion verification matrix

```bash
python -m tools.capture.fos.verify --saves-dir "<private-dir>/fos-saves"
# or:
python tools/capture/fos/verify.py --saves-dir "<private-dir>/fos-saves"
```

## Verification Coverage (87 Converter Keys, Rounds 1–3)

Across Rounds 1–3, the declarative campaigns and 68 assertions in `verify.py`
resolve all **87** `.fos` converter keys (0 unresolved `[hypothesis]` keys):

- **70 confirmed `[observed]`** byte-for-byte layouts across Global Tables 1 & 2,
  `ACHR 0x14`, non-player `ACHR`/`ACRE`, `REFR`, `PGRE`, `CELL`, `QUST`,
  `NPC_`/`CREA` (bits 1, 2, 4, 5, 11, 24), `ECZN`, `TACT`, `MISC`, `FACT`,
  `FLST`, `LVLI`, `REPU`, `CHAL`, `CLAS`, `INFO`, and created `0xFF` records.
- **13 real parser/converter mismatches** reproduced and covered:
  1. `ACHR 0x14` `scale` (`f32 |`) gated on change flag bit 4 (`0x10`).
  2. `ACHR 0x14` bit 22 stores all `SetAV` base overrides (`code < 77`), not
     only skills `32..=45`.
  3. Global Table 1 Type 10 active Pip-Boy radio station & `playback_ms` live
     in the second `vsval` list while the opening header `on` byte stays `0`.
  4. `REFR` `EXTRA_LIST_BITS` must include bit 6 (`0x40`) for `0x21` ownership.
  5. `CELL` bit 3 owner (`ref3 |`) is serialized after bit-31 local-map fog.
  6. Non-player `ACHR` `ModAV` permanent modifiers land under bit 23.
  7. Player `ModAV` radiation (`54`) and hardcore needs (`73..=75`) land in
     `AV2` rather than `AV3`.
  8. `NPC_ 0x7` `Player.SexChange` writes change flag bit 24 (`u8 |`).
  9. Acquired `NOTE`s append to `ACHR 0x14`'s tail notes list with zero
     `TYPE_NOTE` bit-31 records.
  10. `REFR` extra `0x5B` stores a 0-based `DSTD` stage index (`0` on a
      destroyed 1-stage destructible), alongside `0x56` (`f32 |` remaining HP).
  11. `REFR` `EXTRA_LIST_BITS` must include bit 10 (`0x400`) for `0x1C` holder
      entries on picked-up world references.
  12. Item extra `0x3E` (0 payload bytes, emitted by `EquipItem <item> 1`)
      must be skipped cleanly in `parse_item_list()`.
  13. Item extra `0x6E` (`ref3 | u32 |`, paired with `0x39` dropped-from actor)
      is emitted on NPC-dropped world weapon `REFR`s, while active equipped
      player magazine rounds live in `ACHR 0x14` process-block `tok[76]`.
- **3 `[observed]` negative proofs** for Oblivion-era item extras (`0x17`
  secondary worn, `0x26` carried `LIGH` time, `0x28` weapon enchantment charge)
  never emitted by `FalloutNV.exe 1.4.0.525`.

## Optional Synthetic Test-Plugin Generator (`--enable-synthetic-esp`)

Vanilla `FalloutNV.esm` contains zero carryable `LIGH` forms (`0x26`), zero
charged weapon `ENCH` records (`0x28`), and zero `REFR`s carrying `XGLB`
(`0x22`). To test those rare item-extra code paths without distributing any
binary plugin, `harness.py --enable-synthetic-esp` constructs a temporary
clean-room TES4/FNV plugin (`fos_probe_r3.esp`, local FormIDs
`0x01000801..0x01000815` `[design]`) on-demand from public TES4/FNV record
definitions and templates read from the user's own `FalloutNV.esm`, appends it
to `plugins.txt`/`loadorder.txt`, and deletes the plugin while restoring both
load-order files byte-for-byte in `try/finally`.

## `[design]` Harness Defaults

The following parameters are NVForever harness design choices `[design]` (not
original-game format constraints):

- Isolated temporary save subdirectory name `Saves\fosprobe\` under `--ini-dir`.
- Default capture window resolution `1280x720`, script loop tick interval `15`,
  and per-campaign `max_wait` timeouts (`90`–`180` seconds).
- Temporary synthetic plugin filename `fos_probe_r3.esp` and local FormID range
  `0x01000801..0x01000815`.
- Optional defensive pass-through parsing of unemittable item extras `0x17`,
  `0x26`, and `0x28` in `parser.py`.
