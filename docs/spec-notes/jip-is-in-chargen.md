# JIP LN `IsInCharGen`

- [documented: https://geckwiki.com/index.php?title=IsInCharGen,
  https://geckwiki.com/index.php?title=SetInChargen]
  `IsInCharGen` takes no arguments and returns `1` if the player is currently
  in character generation (set via `SetInCharGen 1`), and `0` otherwise.
- [design] `Game::is_in_chargen` (`crates/fnv-game/src/nvse_jip2.rs`) checks
  `PlayerState::in_chargen` (toggled by `SetInCharGen`) as well as open
  character-creation menus (`menus::RACE` with `RaceMenu::Creator`,
  `menus::TAG_SKILLS`, `menus::LOVE_TESTER`, `menus::TRAITS`, and `menus::NAME`
  when not acting as `ShowTextInputMenu`), excluding `RaceMenu::Barber`,
  `RaceMenu::Surgeon`, and ordinary menus such as `menus::BARTER`.
- [observed: Fallout: New Vegas 1.4.0.525 + xNVSE 6.4.9 + JIP LN 57.30 with
  the 10 official base masters, clean-room black-box script probe
  `jip_39_is_in_chargen_without_flag` (`IsInCharGen`/`SetInCharGen`,
  reproducible via `#3678` / PR `#3679`, `tools/capture/nvse/`); no DLL or
  source consulted] Opening `TraitMenu` (`1084`) without `SetInCharGen 1` makes
  `IsInCharGen` return `1` (`c0=0,c1_same=0,c2_next=0,c3_clr=0,traitMM=1084,cgInTrait=1`);
  note `SetInCharGen 1` alone outside a chargen menu left `IsInCharGen` at `0`.
- [hypothesis] Whether the remaining character-creation menus (`RaceMenu::Creator`
  vs `Barber`/`Surgeon`, `menus::TAG_SKILLS`, `menus::LOVE_TESTER`, and
  `menus::NAME`) and ordinary menus (`menus::BARTER`) behave identically without
  `SetInCharGen 1` remains uncaptured.
