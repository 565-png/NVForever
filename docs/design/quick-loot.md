# Quick Loot

A design feature, not a parity target: the original has no such list. It follows Fallout 4's
looting list, which the owner asked for (2026-11-17), in the original's HUD look. Every figure
here is a design choice [design: owner 2026-11-17] unless tagged otherwise.

## 1. What it does

With the NVF Features › Interface row **Quick loot** on (off by default), a container, a body
or, while sneaking, a person under the crosshair shows what it holds in a framed list beside
the crosshair. The game does not stop for it.

| Key (the player's binding) | What it does |
|---|---|
| Activate (E) | takes the selected stack (a pocket: steals it, rolling its chance) |
| Wait (T) | takes everything; not from a pocket, where each item is its own roll |
| Ready Item (R) | opens the full container or pickpocket menu, as Activate would without Quick Loot |
| mouse wheel | moves the selection (up moves it up) |

While the list shows, those keys do nothing else: no reload or holster, no Wait menu, no
camera zoom, scope zoom or hotkey step, no grab (`quick_loot::holds_input`, read by
`control`, `wait`, `weapon_wheel`, `aim_zoom`, `grab`, `activate`). The crosshair's prompt
is not drawn while it shows: the list stands for it.

## 2. What shows it

The activation target (`activate::Target`) is:

- a container that opens (`Action::Open` on a CONT). A locked one keeps its lock prompt;
- a body or an ash pile to search (`Action::Search`); the ash pile lists its actor's items;
- a pocket the player may pick (`Action::Steal` with no refusal,
  `fnv_game::Game::pickpocket_refusal`). One that has caught the player keeps its prompt.

Not on a reference whose script has an OnActivate block, one set off by its activate parents
only, or one that cannot be activated: the game's own activation decides those, so their
prompt stays and E acts as it always did. Not under the weapon wheel, VATS, the character
creator, a kill camera, an idle that holds the camera, or a script's DisablePlayerControls
(movement or rollover text). Not while the HUD is hidden.

## 3. The rules are the container menu's

The list is a `menu::model::MenuModel` opened on the target as the container menu opens it,
the pickpocket's `Steal` (rules, both Sneak skills, its seeded rolls) included. So:

- someone else's container: a take is theft (`crime::ItemsTaken`; karma and witnesses as
  in the menu); the list and its keys are in the crime colour, "Steal" and "Steal All";
- a pocket: each take rolls the stack's chance (`Steal::chance` for the whole stack, the
  figure the list shows); a failure is caught (`pickpocket::Pickpocketed`), the list closes
  and stays closed on that person;
- a body's dropped weapon is a row and is picked up from the ground when taken.

What came in is said as a script's AddItem says it ("Stimpak (2) added"; four or more stacks:
"N items added (M in all)"), and each take has its item's pick-up sound (Take All its own
sound), as in the menu. The rows are read again every half second and after each take.

## 4. Look

Design A of the owner's five (2026-11-17): a dark backing between two HUD-coloured lines with
short ticks at their ends; the name and the selection's place ("3/8") over a rule; up to six
rows, the selection boxed and filled faintly; a short bar above or below where more rows are;
the keys' lines under the box. In the HUD colour (`interface::amber`, recoloured with it), the
crime colour for theft, at the HUD scale (`interface::hud_units`), in the Menu Font row's font
(the game's font 7, or the sharp font, `pause::MenuFont`), redrawn when either changes.

Columns, each with its own row (all off by default, greyed while Quick loot is off): **Show
weight**, **Show value**, **Show value per weight**; a pocket always shows each item's
**chance**. With no column the count stands right-aligned in a column of its own; with any,
it goes beside the name, "Stimpak (2)" [owner 2026-11-17]. The columns are as wide as their
widest figure in the drawn font, so they align in both fonts; a name too long for the list is
cut with "...". The list starts 90 units right of the crosshair and 170 above it, and moves
left when a narrow view has no room for it.

Each drawing checks itself (`quick_loot::overlaps`: texts that meet, a text outside the list
or off its row) and logs `quick loot: drawn "<name>" (… font Original|Sharp …): no overlaps`,
or what overlaps; the unit tests run it for every combination of columns in both fonts.

## 5. Not done

- A controller has no wheel: its D-pad still steps the hotkeys; the selection moves with the
  mouse only.
- MenuMode 1008 scripts (GetMenuTargetRef) and a container's open animation and sounds do not
  run for the list; R's full menu runs them.
