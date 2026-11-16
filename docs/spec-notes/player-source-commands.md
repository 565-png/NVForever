# Player progression and exact reputation source commands

Part of #2271. These entries describe source calls; native game bytecode and
original-game parity remain unverified.

## Documented contract

- [documented: GECK wiki, [AdvancePCLevel](https://geckwiki.com/index.php?title=AdvancePCLevel)]
  `AdvancePCLevel`, also `AdvLevel`, raises the player's level and opens the
  level-up menu. It takes no arguments.
- [documented: GECK wiki, [ModPCAttribute](https://geckwiki.com/index.php?title=ModPCAttribute)]
  `ModPCAttribute StatName value` changes a player SPECIAL attribute permanently;
  it does not change skills. `StatName` is a string and `value` an integer.
- [documented: GECK wiki, [RemoveReputationExact](https://geckwiki.com/index.php?title=RemoveReputationExact)]
  `RemoveReputationExact reputation axis amount` subtracts a possibly fractional
  amount from infamy (axis 0) or fame (axis 1).

## NVForever mapping and open questions

- Advance uses `levelup::advance`, also called by `experience::level_up`, leaving
  XP unchanged. As in `chargen::SpecialSheet::apply`, attribute changes shift the
  base using `actor::shift_base`,
  retaining permanent, temporary and damage pools, carry synchronization and
  actor-value event delivery. [hypothesis] The original's exact attribute pool and
  its XP behavior after manual advancement still need captures.
- The source call follows `revision::keeps`, retaining player attribute allocations
  as `functions::actor::set_actor_value` does. Arguments are read and SPECIAL
  admission is checked before mutation.
- Exact removal uses `functions::reputation::{write_axis,change,store}` for the
  existing reputation writer, limits and trace event. Invalid
  axes retain the established script-return request and call failure. [hypothesis]
  Those neighboring policies also apply to exact removal; negative and zero amounts,
  boundaries, missing scales and disguise interaction require original-game captures.
- [hypothesis: selected portable source convention] These global player commands
  ignore a valid explicit receiver and never redirect the mutation to that receiver.
  An explicit empty/invalid receiver fails before dispatch, rather than being
  reinterpreted as an omitted receiver. Omitted calls work without script self.
  Nonfinite amounts and arguments of the wrong type fail without mutation; integer
  amounts use the established `Args::int` conversion.
- The existing `schemas/script/command_opcodes.json` records 4309, 4368 and 4668
  respectively, with provenance identifying prior original-game command queries.
  They establish command identity, not parameter encoding. This contribution does
  not number the catalogue rows or add a native encoding. Source compilation uses
  the existing named-call IR/SCDA transport. [hypothesis] Native ModPCAttribute enum
  versus string encoding needs compiled GECK output; none is inferred as observed.

## Authored controls and acceptance

- Portable controls in `tests/player_source_commands.rs` cover
  successful compiled source calls, alias, invalid input/receiver, neighboring state,
  existing event consumers and save/restore. They are implementation controls, not
  original-game observations. Qualification is revision-specific: execution of the
  current revision is pending; earlier partial results are recorded in
  [PR #2772](https://github.com/LootJunkieAzza/NVForever/pull/2772). The baseline
  comparison remains UNRUN.
- [hypothesis] Original-game captures should compare XP before/after repeated
  advancement, attribute base/permanent/current pools with effects and damage,
  fractional reputation loss across each limit, invalid axes and receivers, and
  save/load. Native GECK encoding, profile and whole-mod acceptance are UNRUN.

## Compile-only GECK capture plan

[hypothesis: proposed capture, not executed] In a disposable owner-controlled
plugin outside the game directory, compile each of these separate script bodies
with the normal GECK command compiler. Do not execute them as part of the encoding
capture:

```text
scn NVFPlayerSourceAdvanceProbe
Begin GameMode
 AdvancePCLevel
End

scn NVFPlayerSourceAttributeProbe
Begin GameMode
 ModPCAttribute Strength 1
End

scn NVFPlayerSourceReputationProbe
Begin GameMode
 RemoveReputationExact RepNVGoodsprings 0 0.5
End
```

[hypothesis: proposed capture] Retain the compiler/version result, authored source
hash and the resulting SCHR/SCRO/SCDA payloads privately. Repeat attribute compilation
for Luck and a negative integer; repeat reputation compilation for fame and a
fractional amount supplied through a variable. Compare only the compiled data,
without reading executable instructions or plugin implementation tables. Existing
repository script readers can extract the records. Add an observed encoding only
after an independent reader agrees with the GECK output. A separate read-only query
job can repeat `GetCommandOpcode` for the three names and `AdvLevel`; that query
does not settle argument encoding or function behavior.

The authored `tools/smoke/fixtures/player-source-commands.steps.json` runtime-query
fixture and `player-source-commands` smoke scenario
exercise NVForever source compilation through the actual launcher. They are
UNRUN. Their source and printed deltas are portable implementation expectations,
not original-game evidence or native bytecode goldens.
