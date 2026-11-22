# Base Object Swapper (`Data/BaseObjectSwapper/*.ini`) loader and placement swapper

NVForever supports data-driven `Data/BaseObjectSwapper/*.ini` object swap packs
natively without requiring a native DLL plugin (`crates/fnv-world/src/bos.rs`).

## 1. File discovery and precedence

- `[design]` INI packs are discovered under `Data/BaseObjectSwapper/` (and layered mod
  roots via the engine VFS), matching `*.ini` case-insensitively and ignoring non-`.ini`
  files (such as mod-manager marker files).
- `[design]` Discovered `.ini` files are applied in deterministic sorted filename order
  (case-insensitive). When a higher-precedence mod root supplies an `.ini` file with the
  same name, VFS / root layering lets it override the lower-precedence copy.
- `[design]` When `Data/BaseObjectSwapper/` is absent or yields zero valid rules,
  `BosTable::is_empty()` is `true` and placement resolution takes an immediate no-op fast
  path with zero allocations or hash lookups.

## 2. Clean-room INI grammar

- `[design]` UTF-8 text (stripping an optional leading UTF-8 BOM). Comments begin at `;`
  and run to the end of the line. Blank lines are ignored.
- `[design]` Supported section headers (case-insensitive):
  - `[Forms]` — swaps placed references matching the original base form. Also the default
    section before any header so headerless `.ini` packs load cleanly.
  - `[References]` — swaps a specific placed reference by its reference FormID.
  - Other section headers (such as `[Transforms]`) are skipped safely without aborting
    loading; skipped lines are counted in `BosLoadReport::skipped_unsupported_section`.
- `[design]` Rule lines are pipe-delimited (`orig | swap [| filters [| traits]]`):
  - **Form specifiers** (`orig` and comma-separated entries in `swap`):
    - `0xHEX~Plugin.esm` (case-insensitive plugin name, optional `0x` prefix, 1–8 hex
      digits): maps the lower 24-bit local FormID into the named plugin's active
      load-order slot, verifying the record exists in the active `LoadOrder` (falling back
      to `LoadOrder::to_global` for master-indexed overrides). Missing plugins or unknown
      records are skipped safely and counted in `BosLoadReport`.
    - `EditorID`: case-insensitive lookup against winning record `EDID` subrecords in the
      active `LoadOrder` (indexed lazily only when an `.ini` rule uses a bare EditorID).
  - **Comma-separated replacement list (`swap`)**: `Orig|SwapA,SwapB,...` resolves every
    valid candidate and selects among them deterministically per reference (or using the
    session seed under `chanceR`).
  - **Filters (column 3, optional)**: empty or `NONE` (case-insensitive) means unfiltered.
    Non-`NONE` filter expressions are not yet evaluated and are skipped safely
    (`skipped_unsupported_filter`) rather than falsely applied globally.
  - **Traits (column 4, optional)**: empty or `NONE` means unconditional (`Chance::Always`).
    Supported probability modifiers (case-insensitive, finite percentage clamped to
    `0.0..=100.0`):
    - `chanceS(pct)` — deterministic per-reference roll hashed from `(ref_id, rule_index)`
      via SplitMix64 so a placed reference swaps consistently across reloads.
    - `chanceR(pct)` — session-seeded roll hashed from `(ref_id, rule_index, random_seed)`
      (`0` never fires, `100` always fires).
    - Unknown or malformed trait tokens are skipped safely (`skipped_malformed`).

## 3. Precedence and multi-rule evaluation

- `[hypothesis]` `[References]` rules take precedence over `[Forms]` rules for the same
  placed reference: if any `[References]` rule fires for `ref_id`, its target is used;
  otherwise `[Forms]` rules matching the authored base form are evaluated.
- `[hypothesis]` Multiple rules targeting the same original form (for example variant
  chains with independent `chanceS(pct)` rolls or later INI overrides) are evaluated in
  file/line order using independent per-rule hash indices (`rule_index`), with the last
  fired rule winning. If no probabilistic rule fires, the reference keeps its authored
  base form. Exact native plugin PRNG / tie-breaking order has not been captured from the
  third-party DLL and remains `[hypothesis]`.

## 4. Shared consumer seam

- `[design]` `BosTable` attaches to `fnv_world::WorldIndex` (`WorldIndex::apply_bos`,
  `WorldIndex::resolve_base`, `WorldIndex::placement`), which is shared by:
  - `fnv_sim::refs::RefTable::from_index` (simulation and scripting base-form identity;
    runtime/save state in `World` / `GameSnapshot` layers on top without losing saved
    overrides),
  - `fnv_world::reach::ReachIndex::build` (cell reach bounds),
  - `nvf_play::phys` / `nvf_play::phys::prepare` / `nvf_play::script_draw` (collision
    meshes and script-enabled reference drawing), and
  - `nvforever::world` / `nvforever::imposters` (read-ahead model preparation and scene
    spawning).
- `[design]` All placement consumers therefore resolve the same swapped base FormID by
  construction. Transform-only `[Transforms]` rules and non-`NONE` cell/location filters
  remain open follow-ups.
