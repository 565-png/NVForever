# `SetDefaultOpen` and `GetDefaultOpen`

Part of #2271.

## Documented contract

- [documented: GECK wiki, [GetDefaultOpen](https://geckwiki.com/index.php?title=GetDefaultOpen)]
  `ref.GetDefaultOpen` (`0x10D7`, CTDA condition index `215`, parameterless)
  returns `1` if the calling object reference's default state is open and `0`
  otherwise.
- [documented: GECK wiki, [SetDefaultOpen](https://geckwiki.com/index.php?title=SetDefaultOpen)]
  `ref.SetDefaultOpen defaultOpen:int` sets the calling reference's default open
  state (`1` = open by default, `0` = closed by default), matching the "Open By
  Default" flag on a placed reference.

## NVForever mapping and open questions

- [design] Runtime default-open overrides live in `fnv_sim::World::default_open`
  (persisted in `fnv_sim::Snapshot::default_open`, omitted when empty, pruned
  for deleted created references, and removed when set back to the placement's
  authored `ONAM` `open_by_default` value).
- [design] `World::initial` initializes `RefState::open` from
  `World::default_open(id)` so untouched references and containers reset via
  `World::reset_containers` inherit the override on materialization, and
  `fnv_ai::Ai::path_params` reads `game.world.default_open(id)` for untouched
  locked in-place doors. Calling `SetDefaultOpen` on an already-materialized
  reference updates `default_open` without mutating its live `RefState::open`
  (`SetOpenState` owns live open/close state and animations).
- [design] `GetDefaultOpen` is registered at catalogue opcode `0x10D7` (serving
  scripts, bytecode, CTDA condition index `215`, and console) and validates the
  calling reference without materializing untouched placements. `SetDefaultOpen`
  remains unnumbered in `schemas/script/functions.json` (no native bytecode
  opcode invented) and is dispatched through named source calls
  (`functions::refs::call_source`) and the console (`functions::console::set_default_open`).
  Explicit empty, unknown, or deleted receivers and non-numeric/wrong-arity
  arguments fail cleanly without mutating state.
- [observed: Fallout: New Vegas 1.4.0.525 + xNVSE 6.4.9, 2026-11-24, 10 official base
  masters, isolated clean-room staging, JIP LN 57.30, GSDocMitchellHouse (0x00103DF9),
  Wave 4E black-box probe `w4e_c15_door_default_open_air_killcam_speech`: `door=0->1->0`;
  zero DLL/PDB/source consulted] `SetDefaultOpen` / `GetDefaultOpen` round-trips
  `0 -> 1 -> 0` on unscripted placed door `0x001062AC`.
- [hypothesis] Native GECK bytecode encoding for `SetDefaultOpen`, whether the
  original engine immediately toggles an already-loaded door/container's live
  visual open state on `SetDefaultOpen`, and original-game behavior on non-door /
  non-container references remain uncaptured.
