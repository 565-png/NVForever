# NVSE zero-stub batch: `SetPersistent`, `GetFallTimeRemaining`, `GetCalculatedActorSpread`, `IsItemUnique` / `ToggleItemUnique`

Part of #2272. Core-only implementations in `crates/fnv-game` replacing four
`=> zero` stubs without altering `Game::Session` or `engine/`.

## 1. `SetPersistent` (`crates/fnv-game/src/nvse_gaps.rs`, `nvse_refs.rs`)

- [documented: https://geckwiki.com/index.php?title=SetPersistent]
  `reference.SetPersistent persistent:0/1` sets or clears the persistent record
  flag (`0x400`, `fnv_sim::refs::FLAG_PERSISTENT`) on the calling reference for
  the session.
- [observed: `gaps6` probe `persistent.doc`] `SetPersistent` returns `0`.
- [design] `SetPersistent` records an explicit per-reference boolean in
  `OtherState::persistent_overrides` and keeps bit `0x400` in
  `OtherState::flags` synchronized (starting from the reference's current
  record flags and persistent-group status, while keeping `FormId::PLAYER_REF`
  always persistent). `Game::persistent` checks
  `OtherState::persistent_overrides` first and otherwise falls back to static
  placed flags (`0x400`) or `in_persistent_group`, so unrelated writes to
  `OtherState::flags` via `SetReferenceFlag` or `SetFlagsLow` on a
  persistent-group reference whose static record lacks bit `0x400` cannot
  accidentally clear its persistence. `IsPersistent`, `GetPermanent`,
  `GetReferenceFlag 1024`, `GetFlagsLow`, unloaded-cell walks
  (`GetFirstRefInCell`/`GetNumRefsInCell`/`GetRefsInCell`), `GetFormRefs`, and
  `PersistentRefCount` all stay synchronized with explicit `SetPersistent`
  calls.
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
  https://geckwiki.com/index.php?title=Perk_Entry_Points,
  `crates/fnv-combat/src/tuning.rs`, `crates/fnv-combat/src/spread.rs`,
  `schemas/fnv/objects.json` `WEAP.DNAM.animation_type`]
  `actor.GetCalculatedActorSpread` computes the calling actor's live weapon
  spread from the equipped weapon's skill, Strength/skill requirements, and
  animation type (`DNAM` with live `field_overrides`), actor skill/Strength,
  stance (iron sights/aiming, walking/running, sneaking), crippled arms (1H vs
  2H), and perk entry points (`MODIFY_WEAPON_STRENGTH_REQ`,
  `MODIFY_AIMING_MOVE_SPEED`, `CALCULATE_GUN_SPREAD`), reading the 12 Gun Spread
  Formula GMSTs from the load order with fallback defaults matching
  `crates/fnv-combat/src/tuning.rs` (`SETTINGS`, sourced from GECK wiki
  `Gun Spread Formula (New Vegas)`):
  - `fUnaimedSpreadPenalty = 0.2` `[documented: Gun Spread Formula (New Vegas); crates/fnv-combat/src/tuning.rs]`
  - `fWalkingSpreadPenalty = 0.1` `[documented: Gun Spread Formula (New Vegas); crates/fnv-combat/src/tuning.rs]`
  - `fRunningSpreadPenalty = 0.2` `[documented: Gun Spread Formula (New Vegas); crates/fnv-combat/src/tuning.rs]`
  - `fStandingSpreadPenalty = 0.1` `[documented: Gun Spread Formula (New Vegas); crates/fnv-combat/src/tuning.rs]`
  - `fCrippledArm1HSpreadPenalty = 0.2` `[documented: Gun Spread Formula (New Vegas); crates/fnv-combat/src/tuning.rs]`
  - `fCrippledArm2HSpreadPenalty = 0.4` `[documented: Gun Spread Formula (New Vegas); crates/fnv-combat/src/tuning.rs]`
  - `fCrippledArms1HSpreadPenalty = 0.4` `[documented: Gun Spread Formula (New Vegas); crates/fnv-combat/src/tuning.rs]`
  - `fCrippledArms2HSpreadPenalty = 0.6` `[documented: Gun Spread Formula (New Vegas); crates/fnv-combat/src/tuning.rs; observed: w4d_09_spread_all_anim_types below]`
  - `fWeapStrengthReqPenalty = 0.025` `[documented: Gun Spread Formula (New Vegas); crates/fnv-combat/src/tuning.rs]`
  - `fWeapSkillReqPenalty = 0.01` `[documented: Gun Spread Formula (New Vegas); crates/fnv-combat/src/tuning.rs]` (note: the wiki page reports the original game applies `fWeapStrengthReqPenalty` to the skill shortfall as a bug; matching `fnv_combat::spread::actor_spread`, NVForever uses `fWeapSkillReqPenalty` `[design]`)
  - `fWobbleToSkillConversion = 0.5` `[documented: Gun Spread Formula (New Vegas); crates/fnv-combat/src/tuning.rs]`
  - `fMinGunSpreadValue = 0.01` `[documented: Gun Spread Formula (New Vegas); crates/fnv-combat/src/tuning.rs]`
- [observed: Fallout: New Vegas 1.4.0.525 + xNVSE 6.4.9 + JIP LN 57.30 +
  ShowOffNVSE 184, 2026-11-24, `GSDocMitchellHouse` (`0x00103DF9`), 10 official
  base masters, clean-room read-only console probes in `#3679`; no DLL or
  source consulted]:
  - `w4c_a06b_spread_settled` (`tools/capture/nvse/catalogs/wave4c_extenders.json`):
    `1h(norm=0.60925,nw=0.36175,back=0.14338,rc=0.23587,both=0.32838,cripNW=0.32838)|2h(crip=0.32838,cripNW=0.32838,norm=0.14338)`
    — `SetNoGunWobble 1` does **not** zero `GetCalculatedActorSpread` (remains
    `0.32838` on both 1H and 2H).
  - `w4d_09_spread_all_anim_types` (`tools/capture/nvse/catalogs/wave4d_extenders.json`):
    `t3_10mm:anim=3,n=0.29200,r=0.23587,l=0.14338,b=0.32838 | t4_lasp:anim=4,n=0.13875,r=0.23125,l=0.13875,b=0.32375 | t5_hunt:anim=5,n=0.16881,r=0.35381,l=0.35381,b=0.44631 | t6_plas:anim=6,n=0.14338,r=0.32838,l=0.32838,b=0.42088 | t8_flam:anim=8,n=0.18038,r=0.36538,l=0.36538,b=0.45788 | t10_grn:anim=10,n=0.45038,r=0.44663,l=0.44663,b=0.44663`
    — across settled spread readings:
    1. **1H guns (`anim 3` 10mm Pistol, `anim 4` Laser Pistol):** right arm
       adds `+0.09250` (`0.2`), left arm alone adds `+0.00000`, both arms add
       `+0.18500` (`0.4`).
    2. **2H weapons (`anim 5` Hunting Rifle, `anim 6` Plasma Rifle, `anim 8`
       Flamer):** either single arm (`r` or `l`) adds `+0.18500` (`0.4`), and
       both arms add `+0.27750` (`0.6` = `fCrippledArms2HSpreadPenalty`).
    3. **Thrown (`anim 10` Frag Grenade `0x4330`):** zero crippled-arm spread
       penalty (`+0.00000` for right, left, and both arms).
- [hypothesis] Items without a direct external source citation or original-game
  observation:
  1. **Unprobed weapon animation types (`0..=2`, `7`, `9`, `11..=13`)**:
     `w4d_09_spread_all_anim_types` directly observed types `3, 4` (1H guns),
     `5, 6, 8` (2H ranged), and `10` (thrown). Grouping `7` (Rifle Energy) and
     `9` (Launcher) with `5, 6, 8` (2H ranged), and `0..=2` (unarmed/melee)
     plus `11..=13` (mines/thrown) with `10` (zero crippled-arm penalty) is
     `[hypothesis]`.
  2. **GMST fallback defaults when absent from masters**: per
     `crates/fnv-combat/src/tuning.rs` and `docs/spec-notes/combat-data.md` §3,
     falling back to the wiki GMST values when a load order omits those GMST
     records is `[hypothesis]`.
  3. **ShowOff `GetCalculatedActorSpread` remaining scope/units**: returning the
     actor-spread component in spread units after `CALCULATE_GUN_SPREAD` perks
     (excluding weapon `min_spread`, draw recoil bloom, and ammo spread
     modifiers, and returning `0.0` when unarmed or called without a subject) is
     `[hypothesis]` pending further original-game captures.

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
