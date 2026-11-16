# Key events while the Mods page holds the world

## Contract

- [documented: GECK wiki, `SetOnKeyDownEventHandler` and
  `SetOnKeyUpEventHandler`, also `jip-events.md` §2] The registered function receives
  the physical key's scan code when that key is pressed or released. Disabling a key
  does not prevent its event.
- [documented: `ext-input.md` §1] Wheel directions use codes 264 and 265.
- [observed: installed MCM Extender `OnRestart.gek` and `OnKeyDown.gek`, local demand
  inventory, 2026-11-16] MCM Extender registers wheel and keyboard handlers. Its wheel
  handler reads the open start menu and chooses the list under the cursor; its search
  input also arrives through key events. The files remain private.
- [design: `ui-tile-tree.md` §7; `mcm.md` §2.2] The Mods page stands in for menu 1013
  while the world is held. Its event handlers must see that menu context, and an
  existing message box must remain open.

## Engine boundary

[design] Each Mods-page frame advances the existing JIP physical-key edge tracker
before its menu script blocks. The engine uses the same dispatcher and held-key
state as an ordinary frame. A held key produces one down event, its release one up
event, and returning to play must not replay a page's already-delivered edge. This
does not run world updates, advance game time, or dispatch unrelated queued events.
The surrounding page menu context is restored after the frame.

[documented: `jip-ui-9.md` §1, `GetCursorPos` / `SetCursorPos`] XML cursor queries
use the tile tree's pointer in a menu, and cursor writes are admitted in menu mode
or while the HUD cursor is enabled. [design] The held StartMenu page has the same
cursor admission. Its XML coordinates use the already transformed, hit-tested
pointer; raw window pixels still serve non-XML queries. Plain-play behavior and
ordinary menu behavior keep their existing paths. [observed: installed MCM
Extender `OnKeyDown.gek`, local demand inventory, 2026-11-16] Its wheel callback
selects the list by querying XML cursor coordinates and the list's bounds.

[observed: `menu-xml.md` §10.1, original-game pause-menu capture] The active menu
getter returns 1013 while `MenuMode 1013` holds there.
[observed: installed MCM Extender `InputHandler.gek`, `MenuSearchInput.gek`
and `OnButtonDown.gek`, local demand inventory, 2026-11-16] These input handlers
require active menu 1013. [design] When no ordinary menu overlays the held StartMenu
page, that getter returns StartMenu. An existing message menu keeps precedence;
the page is a context for its blocks, not a replacement for the overlay.

[hypothesis] Exact ordering against the original game's menu scripts still needs a
capture. Dispatch before menu blocks preserves NVForever's ordinary-frame ordering.

## Validation and limits

[design] Authored regressions must register handlers through the source compiler,
feed physical wheel/keyboard input through the engine input publisher, and call the
same Mods-page tick as the pause page. Check a list trait written by the handler,
menu context, held/released keys, disabled keys, repeated entry, and frozen game
time. A separate source-registered wheel callback must route a list update using
distinct raw and hit-tested cursor positions and request a cursor warp in that
same page context. A getter-only test cannot establish this boundary.

[hypothesis] Full installed MCM 1.5.1 + MCM Extender + Vanilla UI Plus acceptance
remains open until their original initialization, menu registration, list scrolling,
option changes and save/load run on the selected installed profile. The authored
tests do not establish native parity or complete compatibility for those packages.
