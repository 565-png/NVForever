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
- [observed: 2026-11-23, `GSDocMitchellHouse` (`0x00103DF9`), fresh game,
  Fallout: New Vegas 1.4.0.525 + xNVSE 6.4.9 + JIP LN 57.30 with the 10
  official base masters, clean-room black-box script probe
  `jip_39_is_in_chargen_without_flag` (`IsInCharGen`/`SetInCharGen`/`ShowTraitMenu`,
  reproducible via `#3678` / PR `#3679`,
  `tools/capture/nvse/catalogs/wave4a_extenders.json`); no DLL or source
  consulted] Calling `SetInCharGen 1` (checked same-frame and after settling,
  then `SetInCharGen 0`) outside a chargen menu left `IsInCharGen` at `0`,
  whereas opening `TraitMenu` (`ShowTraitMenu`, menu mode `1084`) without
  `SetInCharGen 1` made `IsInCharGen` return `1`
  (`c0=0,c1_same=0,c2_next=0,c3_clr=0,traitMM=1084,cgInTrait=1`). The
  contradiction where `SetInCharGen 1` alone left `IsInCharGen == 0` (against
  the documented wiki page and NVForever's `PlayerState::in_chargen` check) is
  tracked as a dedicated follow-up bug issue.
- [hypothesis] Whether the remaining character-creation menus (`RaceMenu::Creator`
  vs `Barber`/`Surgeon`, `menus::TAG_SKILLS`, `menus::LOVE_TESTER`, and
  `menus::NAME`) and ordinary menus (`menus::BARTER`) behave identically without
  `SetInCharGen 1` remains uncaptured.
