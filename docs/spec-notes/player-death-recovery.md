# Player death recovery

[observed] The original-game campaign `o29_dmg` (2026-11-08, issue #1858)
reports that player death reloads the save, and warns that a pending capture job
can therefore repeat after each reload. The recorded method and limits are in
[observer-checks.md](../handoff/observer-checks.md), item 89. That observation
establishes a reload, not the exact slot ordering, menu treatment or clock below.

[documented] GECK's [Complete List of GameSettings](https://geckwiki.com/index.php?title=Complete_List_of_GameSettings)
lists `fPlayerDeathReloadTime` with default `5.000000`. This leaf reads the live
loaded/runtime GMST through `World::setting_f32`; it does not alter game data.

[hypothesis] Engineering policy: a finite nonnegative setting takes precedence;
a missing, negative or nonfinite value uses five seconds. Count real seconds
only while the world is playing and unpaused, beginning after the first observed
dead frame. The death camera's slow-motion multiplier does not slow this timer.
A script resurrection cancels the attempt even while a menu is open. These clock
and cancellation details require original-game timing confirmation.

[hypothesis] Engineering policy: attempt recovery once per uninterrupted death.
Read only the current NVForever profile's own save directory, ordered by the
existing save-header timestamp, without preferring manual over quick, auto or
system slots. Use the existing full format/plugin/remap/body preflight, skip
unreadable or incompatible files, and skip bodies whose player is already dead.
This avoids automatically reloading an already-dead player. It cannot prove that
a later script will not kill a living saved player again. No original `.fos` save
is imported by this feature.

[hypothesis] Engineering policy: open the existing Load menu before sending the
existing `LoadRequest`. Leave it open when no living usable save exists or a
handoff cannot start; do not continuously dispatch requests. An already-pending
load is retained without adding another. Use the existing child-process/window
handoff, not a second restart mechanism. No save is written by death recovery;
the existing refusal to save a dead player and queued-autosave handling remain.

[observed] Ten authored CPU controls passed in an isolated normal engine context
based on main26e plus six UI ports. They use portable authored records, real
`Game::kill`/`resurrect`, actual NVForever save files, ECS time/menu resources and
`LoadRequest` messages. They cover setting changes/defaults, pause/resurrection,
manual/quick/auto ordering, one dispatch, invalid/dead save fallback, absent or
unreadable profiles, pending loads and preservation of an existing autosave.
These are historical CPU results, not a build or run of this public7201 port.

[documented: current port qualification] Current-main compilation, strict Clippy, rendered death/menu recovery
and child-window handoff remain unverified. The `death-recovery-handoff` smoke
scenario is authored and UNRUN. The existing camera-only checks keep every original acceptance condition, including
`death-fade` black at six seconds. Their isolated fixture homes apply the loaded
`fPlayerDeathReloadTime` setting as 120 seconds before killing the player and
require the real successful console result; this prevents recovery from ending
the camera/float/music observation window. Missing GMST admission is a visible
fixture failure. No parity golden or acceptance range is relaxed.


[documented: NVForever 7201ca6d57d3bb8a435bffc3fc7a5ecedf79dea3,
`crates/fnv-game/src/functions/console.rs` set_game_setting and
`crates/fnv-sim/src/world.rs` setting_f32/change_setting] Existing `setgs` mutates
only an actual loaded setting for the session. The recovery consumer reads that
same live value. The 120-second camera-only fixture input is an engineering test
setup; it is not an original-game timing observation or a production delay change.
The separate `death-recovery-handoff` scenario retains its recovery behavior and
all expectations. Rendered runs remain UNRUN; source preparation and historical
CPU acceptance do not establish child-window handoff or no-save menu usability.
An early Draft may disclose these pending checks without waiting for them.
