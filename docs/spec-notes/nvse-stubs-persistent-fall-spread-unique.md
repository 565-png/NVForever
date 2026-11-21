# NVSE zero-stub batch: `SetPersistent`, `GetFallTimeRemaining`, `GetCalculatedActorSpread`, `IsItemUnique` / `ToggleItemUnique`

Part of #2272. Core-only implementations in `crates/fnv-game` replacing four
`=> zero` stubs without altering `Game::Session` or `engine/`.

## 1. `SetPersistent` (`crates/fnv-game/src/nvse_gaps.rs`, `nvse_refs.rs`)

- [documented: https://geckwiki.com/index.php?title=SetPersistent]
  `reference.SetPersistent persistent:0/1` sets or clears the persistent record
  flag (`0x400`, `fnv_sim::refs::FLAG_PERSISTENT`) on the calling reference for
  the session.
- [observed: `gaps6` probe `persistent.doc`] `SetPersistent` returns `0`.
- [design] `SetPersistent` updates bit `0x400` in `OtherState::flags` (starting
  from the reference's current record flags and persistent-group status, while
  keeping `FormId::PLAYER_REF` always persistent). `Game::persistent` reads
  `OtherState::flags` before falling back to static placed flags/group so
  `IsPersistent`, `GetPermanent`, `GetReferenceFlag 1024`, `SetReferenceFlag`,
  `GetFlagsLow`/`SetFlagsLow`, `GetFirstRefInCell`/`GetNumRefsInCell`/`GetRefsInCell`
  on unloaded cells, `GetFormRefs`, and `PersistentRefCount` all stay
  synchronized.
- [hypothesis] Persistence changes made by `SetPersistent` are session-only and
  do not persist across save/load; `FormId::PLAYER_REF` ignores attempts to
  clear its persistent flag.

## 2. `GetFallTimeRemaining` (`crates/fnv-game/src/nvse_jip11.rs`)

- [documented: https://geckwiki.com/index.php?title=GetFallTimeRemaining]
  `actor.GetFallTimeRemaining` returns the estimated remaining fall time in
  seconds before the calling actor reaches the ground, and `0` when grounded.
- [design] `Jip11State::fall_time_remaining` and `Game::set_fall_time_remaining`
  allow an engine caller to supply a direct per-actor remaining fall time. When
  no direct value is present and the actor is grounded (`air_times <= 0`),
  `Game::fall_time_remaining` returns `0.0`.
- [hypothesis] When an actor is airborne (`gaps.air_times > 0`) and no direct
  engine value is supplied, `Game::fall_time_remaining` estimates remaining time
  by solving `0.5 * g_eff * t^2 - vz * t - height = 0` from a downward
  `CastKind::World` ray cast (`crate::ray::cast`), tracked vertical velocity
  (`jip2.velocity`, falling back to `-g_eff * air_time` when untracked), world
  gravity (`686.7` units/s² = 9.81 m/s² × 70 units/m, or `SetLocalGravityVector`
  when active), and `SetActorGravityMult`. `IsInAir` is untouched.

## 3. `GetCalculatedActorSpread` (`crates/fnv-game/src/nvse_showoff4.rs`)

- [documented: https://geckwiki.com/index.php?title=GetCalculatedActorSpread,
  https://geckwiki.com/index.php?title=Gun_Spread_Formula_(New_Vegas),
  https://geckwiki.com/index.php?title=Perk_Entry_Points]
  `actor.GetCalculatedActorSpread` computes the calling actor's live weapon
  spread from the equipped weapon's skill, Strength/skill requirements, and
  animation type (`DNAM` with live `field_overrides`), actor skill/Strength,
  stance (iron sights/aiming, walking/running, sneaking), crippled arms (1H vs
  2H), `SetNoGunWobble`, and perk entry points (`MODIFY_WEAPON_STRENGTH_REQ`,
  `MODIFY_AIMING_MOVE_SPEED`, `CALCULATE_GUN_SPREAD`), reading all Gun Spread
  Formula GMSTs (`fUnaimedSpreadPenalty`, `fWalkingSpreadPenalty`,
  `fRunningSpreadPenalty`, `fStandingSpreadPenalty`,
  `fCrippledArm1HSpreadPenalty`, `fCrippledArm2HSpreadPenalty`,
  `fCrippledArms1HSpreadPenalty`, `fCrippledArms2HSpreadPenalty`,
  `fWeapStrengthReqPenalty`, `fWeapSkillReqPenalty`,
  `fWobbleToSkillConversion`, `fMinGunSpreadValue`) from the load order with
  documented defaults.
- [hypothesis] Because ShowOff's wiki page for `GetCalculatedActorSpread` is
  marked Incomplete, returning the actor-spread component (in spread units,
  after `SetNoGunWobble` and `CALCULATE_GUN_SPREAD` perks, excluding weapon
  `min_spread` and ammo spread modifiers, and `0.0` when unarmed or called
  without a subject) is a hypothesis pending original-game capture.

## 4. `IsItemUnique` / `ToggleItemUnique` (`crates/fnv-game/src/nvse_plugins.rs`, `nvse_gaps.rs`, `repair.rs`)

- [documented: https://geckwiki.com/index.php?title=ToggleItemUnique,
  https://geckwiki.com/index.php?title=IsItemUnique]
  `ToggleItemUnique item/list:form doToggle:1/0` marks or un-marks a single item
  form or every member of a `FLST` form list as unique for the session so it
  does not appear as an optional repair-part source in the repair menu;
  `IsItemUnique item:form` returns `1` if marked unique and `0` otherwise.
- [observed: `data3` probes `unique.stimpak`, `unique.pistol`, `unique.toggle`,
  `unique.restore`] Non-repairable forms such as `ALCH` ingestibles (stimpak)
  ignore `ToggleItemUnique` and continue reading `0` from `IsItemUnique`.
- [design] Session state is stored in `GapsState::unique_items` (keeping
  `Game::Session` untouched), restricted to `WEAP` and `ARMO` base forms (and
  `FLST` members resolving to `WEAP`/`ARMO`). `Game::is_item_unique` exposes the
  flag, and `Game::player_repair_list` (`crates/fnv-game/src/repair.rs`) filters
  out unique items as repair-part sources while still allowing a unique item to
  be repaired as the target using non-unique donors.
- [hypothesis] Whether non-`WEAP`/non-`ARMO` forms beyond `ALCH` are also
  ignored in the original game, and whether unique status persists across
  save/load, remain hypothesis pending capture.
