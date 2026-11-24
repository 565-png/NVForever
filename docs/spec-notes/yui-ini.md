# yUI INI configuration (`Data\NVSE\Plugins\yUI.ini`)

**Clean-room provenance:** Only the public **yUI - User Ynterface** v1.4d mod page
(`https://www.nexusmods.com/newvegas/mods/74357`), shipped `Data\NVSE\Plugins\yUI.ini`
(section names, key names, defaults, and inline comments), and **ySI - Sorting Ycons**
1.4b (`https://www.nexusmods.com/newvegas/mods/74358`) shipped `Data\menus\ySI\ySI.ini`
were consulted. No plugin DLL (`yUI.dll`), PDB, or third-party C++ source code was
opened, disassembled, or consulted.

## VFS discovery, layering, and vanilla-unchanged gate

(`crates/fnv-game/src/yui_ini.rs`, wired into `crates/fnv-game/src/ysi_filename.rs`,
`crates/fnv-game/src/nvse_small.rs`, `crates/fnv-game/src/ui_loading.rs`,
`engine/nvf-play/src/menu.rs`, `engine/nvf-play/src/menu/model.rs`,
`engine/nvf-play/src/hotkeys.rs`, `engine/nvf-play/src/cursor.rs`,
`engine/nvf-play/src/tiles/item_mod_menu.rs`, and
`engine/nvf-play/src/tiles/repair_menu.rs`.)

- NVForever reads `NVSE\Plugins\yUI.ini` case-insensitively through `mod_files` / VFS
  [documented: yUI - User Ynterface v1.4d (`https://www.nexusmods.com/newvegas/mods/74357`),
  shipped `Data\NVSE\Plugins\yUI.ini` install path] and `menus\ySI\ySI.ini` when
  present [documented: ySI - Sorting Ycons 1.4b
  (`https://www.nexusmods.com/newvegas/mods/74358`), shipped `Data\menus\ySI\ySI.ini`
  containing `[General] bSortingIcons = 1`]. Effective `[General] bSortingIcons`
  combines both files with logical OR (`yUI.bSortingIcons || ySI.bSortingIcons`) so
  `ySI.ini bSortingIcons = 0` does not turn off an explicit `yUI.ini bSortingIcons = 1`,
  and `yUI.ini bSortingIcons = 0` with no `ySI.ini` disables UI injection while leaving
  `ySIGetTrait` JSON queries active [observed: Fallout: New Vegas 1.4.0.525 + xNVSE
  6.4.9, 2026-11-24, 10 official base masters, isolated clean-room staging, yUI 140 +
  `yUI.ini` + `ySI.ini` + `ySI.xml` + `ySI.json`, GSDocMitchellHouse (`0x00103DF9`),
  Wave 4E black-box probe `w4e_05_ysi_ini_precedence_and_gating`:
  `yUI0+none(trait=(fba1),ui=-999)|yUI0+ySI1(ui=1)|yUI1+ySI0(trait=(fba1),ui=1)`; zero
  DLL/PDB/source consulted].
- When neither INI file is present, `YuiConfig::load_with` returns `None`: unmodded
  vanilla menus, item ordering, hotkey icons, and amber cursor tint remain completely
  unchanged, while standalone `menus\ySI\ySI.xml` / `menus\ySI\ySI.json` / `ySISetTrait`
  sessions keep their existing behavior [design].
- Parsing operates strictly on raw `&[u8]` slices (stripping optional UTF-8 BOM, CRLF/LF
  endings, `;`/`#` comments, ASCII case-insensitive section/key matching, and finite
  numeric or `true`/`false`/`yes`/`no`/`on`/`off` boolean parsing) [design].
- This slice does **not** raise `GetPluginVersion "yUI"` (`docs/spec-notes/ext-version.md`
  remains truthful).

## Wired sections and keys

Every stored field in `YuiConfig` has a live engine consumer:

- `[General] bSortingIcons` (default `0` in `yUI.ini`; `1` in `menus\ySI\ySI.ini`):
  master gate controlling `[Sorting and Icons]` and ySI UI XML injection [documented:
  yUI v1.4d `Data\NVSE\Plugins\yUI.ini` (`; enable Sorting and Icons section which
  controls ySI`)]. When `yUI.ini` is present and effective `bSortingIcons = 0` (neither
  `yUI.ini` nor `ySI.ini` sets it to `1`), `ui_loading` skips `menus\ySI\ySI.xml`
  injection and `Game::ysi_active()` returns `false`, disabling ySI inventory sorting,
  row icons, and hotkey icon replacement while keeping `ySIGetTrait`/`ySISetTrait` JSON
  queries active [observed: Wave 4E `w4e_05_ysi_ini_precedence_and_gating` above;
  design: `Game::ysi_active()` menu gate].
- `[General] bMatchingCursor` (default `0` in `yUI.ini`): tints the game mouse pointer
  (`textures\interface\icons\misc\cursor.dds`) to the active HUD colour
  (`nvf_play::interface::hud_rgb()`) instead of fixed `AMBER` and re-tints live if the
  HUD colour changes, composing cleanly with SUP `SetCursorTrait` scaling/tinting
  [documented: yUI v1.4d `Data\NVSE\Plugins\yUI.ini` (`; match cursor color to HUD color`);
  design: native `cursor.rs` luminance re-tinting from `interface::hud_rgb()`].
- `[Sorting and Icons] bSortInventory` (default `1`, gated by `bSortingIcons`): controls
  whether `MenuModel::apply_ysi_options` re-sorts inventory, container, and barter item
  rows by ySI sorting tag before alphabetical name order [documented: yUI v1.4d
  `Data\NVSE\Plugins\yUI.ini` (`; sort inventory according to tag names supplied in .json`)].
  When `0`, rows keep vanilla alphabetical / `SortBy` order while still allowing
  `bAddInventoryIcons` to attach row icons [design].
- `[Sorting and Icons] bAddInventoryIcons` (default `1`, gated by `bSortingIcons`):
  controls whether `sync_ysi_model` (Inventory, Container, Barter) and `item_mod_menu` /
  `repair_menu` attach `YsiItemDisplay` icon metadata to item rows and invoke
  `ui_ysi::apply_row_icon` in `tiles/item_menu.rs`, `tiles/item_mod_menu.rs`, and
  `tiles/repair_menu.rs` [documented: yUI v1.4d `Data\NVSE\Plugins\yUI.ini` (`; add ycons
  to inventory, container and barter menus`); design: also gates Repair and ItemMod tile
  menus when ySI is active]. When `0` with `bSortInventory = 1`, rows still sort by ySI
  tag and `apply_row_icon` is skipped (unloading any existing `ySIImage` child) so zero
  `ySIImage` / `ySINoTag` tiles are instantiated while `<Menu>/ySI` remains readable by
  scripts [design].
- `[Sorting and Icons] bReplaceHotkeyIcons` (default `1`, gated by `bSortingIcons`):
  when enabled and an item has a resolved ySI icon (from session `ySISetTrait` overrides
  or `menus/ySI/*.json`), `nvf_play::hotkeys::icon_override` falls back to
  `Game::ysi_hotkey_icon_path` whenever no explicit script `SetItemHotkeyIconPath`
  override is set, updating both the Pip-Boy hotkey panel (`pipboy/hotkey_panel.rs`) and
  the weapon wheel (`weapon_wheel/draw.rs`) [documented: yUI v1.4d
  `Data\NVSE\Plugins\yUI.ini` (`; replace hotkey icons with ycons`); design:
  `SetItemHotkeyIconPath` retains precedence over ySI default icons, and when `yUI.ini`
  is absent hotkey icon replacement stays off].

## Already-avoided / intentionally unbuilt keys

Skipped during parsing with no dead struct fields:

- `[General] bFixIndefiniteSorting`, `bFixDroppedItems`, `bFixTablineSelected`:
  original-FNV UI bugs (`sort_rows` already uses a deterministic total order including
  condition bits, `ContainerMenu` lists all rows, and `InventoryMenu` tabline tracks only
  the active tab natively in NVForever) [documented: yUI v1.4d
  `Data\NVSE\Plugins\yUI.ini` comments].
- `[Sorting and Icons] bEnableCategories`: JSON-defined clickable inventory category
  folders (`category`/`tab`/`count` entries in `menus/ySI/*.json` replacing the vanilla
  keyring) are not built in this slice; NVForever keeps its native Misc `Keyring` and
  `Caravan Cards` folders until a dedicated categories PR is implemented.
