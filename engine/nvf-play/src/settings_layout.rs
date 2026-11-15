//! Where the settings page shows each row: its tab, and the category (a heading) it stands
//! under in that tab (#1953, the menu overhaul; docs/design/settings-page.md). Presentation
//! only: a row keeps the key, the note, the default and the group it registered with
//! (`crate::settings`), and the registry keeps its order, which the values, the INI and the
//! test aids go by. This table only says where the page draws it [design].
//!
//! **Eight tabs, the original's four categories first** [design: #1953; the original's
//! Settings opens Gameplay, Display, Audio and Controls, observed in the original's pause
//! menu, 2026-11-09; the owner's menu overhaul, 2026-11-15]: Gameplay, Display, Audio,
//! Controls, then Graphics, NVF Features, Tweaks and Debug ([`TAB_ORDER`]). Each tab lists
//! its [`SECTIONS`] in order, each a **category**: a heading the player can open and close,
//! an empty line after each open one ([`ResolvedLayout::tab_entries`]).
//!
//! **The Controls tab's own rows** (`crate::settings_view::ControlRow`: the input device,
//! each control's key or button, Invert Y, the sensitivities, Rumble) are named here too, by
//! [`CONTROL_PREFIX`] and their name (`controls:device`, `controls:bind:Jump`), beside the
//! registered rows of the same category (Button Style under Input) [design: the overhaul].
//!
//! **NVF Features** holds only what the owner names (`NVF_FEATURES_KEYS`, a test keeps
//! it so) [design: owner 2026-11-15: "do not move any feature there without my say so"].
//!
//! **A row the table does not name** stays under the group it registered with, used as its
//! tab with no heading, after the tab's named rows, in registry order; a group that is
//! none of the tabs becomes a tab of its own after Debug ([`resolve_layout`]). The engine's
//! own rows are all named (a test keeps it so); the fallback is for what a later change
//! adds before it places its row.
//!
//! [`Placement::subgroup`] is the Tweaks tab's group (Gameplay, Combat, … as Stewie's
//! Tweaks sorts them): the tab shows one group at a time, chosen by its Category row
//! (`crate::settings_view`), its categories closed until opened [design: owner 2026-11-15].
//!
//! What a row's visibility is (the colour sliders only for Custom, the controller rows only
//! with a controller, a row its module hides) is not here: the page decides it each frame
//! (`crate::settings_view`), so a hidden row keeps its tab and category.

use crate::settings::Setting;

/// Where a row is shown: its tab, the Tweaks tab's group, the category (heading) over it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Placement {
    pub tab: &'static str,
    /// The Tweaks tab's group (`None` on every other tab).
    pub subgroup: Option<&'static str>,
    pub heading: Option<&'static str>,
}

impl Placement {
    /// A tab of its own, with no heading.
    pub const fn tab(tab: &'static str) -> Placement {
        Placement { tab, subgroup: None, heading: None }
    }

    /// Its tab, group and heading in English joined by " > " (`Gameplay > Dialogue`), as
    /// the log names it.
    pub fn path(&self) -> String {
        [Some(self.tab), self.subgroup, self.heading].into_iter().flatten().collect::<Vec<_>>().join(" > ")
    }

    /// A category of the Tweaks tab (closed until the player opens it) [design: owner
    /// 2026-11-15].
    pub fn closed_by_default(&self) -> bool {
        self.tab == TWEAKS_TAB
    }
}

/// A run of rows under one heading: registered rows by their key, the Controls tab's own by
/// [`CONTROL_PREFIX`] and their name.
pub struct Section {
    pub placement: Placement,
    pub keys: &'static [&'static str],
}

/// The tabs, in the order the page lists them [design: #1953; the overhaul].
pub const TAB_ORDER: [&str; 8] = ["Gameplay", "Display", "Audio", "Controls", "Graphics", NVF_FEATURES_TAB, TWEAKS_TAB, "Debug"];
/// The tab whose own rows (`crate::bindings`) the table names by [`CONTROL_PREFIX`].
pub const CONTROLS_TAB: &str = "Controls";
/// The tab #1892's tweaks register under.
pub const TWEAKS_TAB: &str = "Tweaks";
/// NVForever's own features the owner names [design: owner 2026-11-15].
pub const NVF_FEATURES_TAB: &str = "NVF Features";
/// A Controls row in the table: `controls:` and its name (`crate::settings_view::ControlRow`).
pub const CONTROL_PREFIX: &str = "controls:";

const fn section(tab: &'static str, heading: Option<&'static str>, keys: &'static [&'static str]) -> Section {
    Section { placement: Placement { tab, subgroup: None, heading }, keys }
}

/// The rows the NVF Features tab may hold: only what the owner named there [design: owner
/// 2026-11-15]. Adding a key needs the owner's word; a bigger gameplay change is offered to
/// the owner for this tab, never moved here unasked (docs/design/settings-page.md).
pub const NVF_FEATURES_KEYS: [&str; 8] = [
    "advanced-movement",
    "movement-inertia",
    "weapon-handling",
    "advanced-recoil",
    "weapon-inertia",
    "animation-smoothing",
    "camera-motion",
    "grass-fire",
];

/// Every named row's tab and category, in the order the page shows them [design: the
/// owner's menu overhaul, 2026-11-15, tab by tab; docs/design/settings-page.md §2]. A key
/// not registered in a run is left out (`resolve_layout`), so rows of features still to
/// come (the movement rows, #1892's tweaks) can have their place already.
pub const SECTIONS: &[Section] = &[
    section("Gameplay", Some("Game"), &["difficulty", "hardcore", "reload-slows", "UsePipBoy"]),
    // "V.A.T.S." as the data spells it (MESG `HelpVATS`).
    section("Gameplay", Some("V.A.T.S."), &["vats-style", "vats-slowdown", "kill-camera"]),
    section("Gameplay", Some("Camera"), &["detached-camera", "true-iron-sights", "first-person-body", "weapon-sway", "head-bob", "weapon-lower", "field-of-view", "viewmodel-field-of-view"]),
    section("Gameplay", Some("Dialogue"), &["dialogue-zoom", "dialogue-look", "dialogue-recentre", "menu-pause", "dialogue-goodbye-hint", "lip-sync"]),
    section("Gameplay", Some("Saving"), &["autosave-travel", "autosave-fast-travel", "autosave-wait", "autosave-sleep", "autosave-scripts"]),
    section("Gameplay", Some("Bug Reports"), &["send-reports", "reports-delete"]),
    section("Display", Some("Screen"), &["monitor", "resolution", "borderless"]),
    section("Display", Some("Frame Rate"), &["display-sync", "frame-cap", "physics-rate"]),
    section("Display", Some("Upscaling and Anti-Aliasing"), &["upscaler", "upscaler-quality", "dlss-model", "fsr-library", "anti-aliasing", "sharpen"]),
    section("Display", Some("Frame Generation and Latency"), &["frame-generation", "frame-generation-frames", "latency-reduction"]),
    section("Display", Some("Interface"), &["language", "sneak-gauges", "dynamic-crosshair", "crosshair-style", "ui-scale", "menu-scale"]),
    section("Display", Some("Colours"), &["hud-colour", "hud-red", "hud-green", "hud-blue", "pipboy-colour", "pipboy-red", "pipboy-green", "pipboy-blue"]),
    section(
        "Display",
        Some("Performance Overlay"),
        &[
            "fps-counter",
            "fps-counter-fps",
            "fps-counter-graph",
            "fps-counter-split",
            "fps-counter-cpu",
            "fps-counter-gpu",
            "fps-counter-vram",
            "fps-counter-ram",
            "fps-counter-loaded",
            "fps-counter-upscaling",
            "fps-counter-low",
        ],
    ),
    section("Audio", Some("Volume"), &["volume-master", "volume-music", "volume-footstep", "volume-voice", "volume-effects", "volume-radio"]),
    section("Audio", Some("Sound"), &["music", "voice", "reverb", "voice-reverb"]),
    section("Controls", Some("Input"), &["controls:device", "button-style", "controls:automatic-input", "controls:rumble"]),
    section("Controls", Some("Look"), &["controls:invert", "controls:sensitivity", "controls:vertical", "controls:aim-sensitivity"]),
    section("Controls", Some("Aim Assist"), &["aim-assist", "aim-slowdown", "aim-lock"]),
    section(
        "Controls",
        Some("Movement"),
        &[
            "controls:bind:Forward",
            "controls:bind:Back",
            "controls:bind:Slide Left",
            "controls:bind:Slide Right",
            "controls:bind:Jump",
            "controls:bind:Crouch/Sneak",
            "controls:bind:Run",
            "controls:bind:Always Run",
            "controls:bind:Auto Move",
        ],
    ),
    section("Controls", Some("Combat"), &["controls:bind:Use", "controls:bind:Block", "controls:bind:Ready Item", "controls:bind:Vats", "controls:bind:Ammo Swap", "controls:bind:Lower Weapon"]),
    section(
        "Controls",
        Some("Actions"),
        &["controls:bind:Activate", "controls:bind:Grab", "controls:bind:Menu Mode", "controls:bind:Rest", "controls:bind:Toggle POV", "controls:bind:QuickSave", "controls:bind:QuickLoad"],
    ),
    section(
        "Controls",
        Some("Hotkeys"),
        &["controls:bind:Hotkey1", "controls:bind:Hotkey3", "controls:bind:Hotkey4", "controls:bind:Hotkey5", "controls:bind:Hotkey6", "controls:bind:Hotkey7", "controls:bind:Hotkey8"],
    ),
    section("Graphics", Some("Quality"), &["quality", "enhanced"]),
    section("Graphics", Some("Shadows"), &["sun-shadows", "shadow-filter", "shadow-distance", "fine-shadows"]),
    section("Graphics", Some("Lighting and Effects"), &["ambient-occlusion", "light-shafts", "bloom", "grade", "glow", "eye-adaptation", "neon-glow"]),
    section("Graphics", Some("Water"), &["water-reflections", "water-mirror"]),
    section("Graphics", Some("Detail and Distance"), &["distant-lod", "grass", "anisotropic", "terrain-height-blend", "object-distance", "actor-distance", "item-distance", "grass-distance"]),
    section("Graphics", Some("World and Animation"), &["head-tracking", "foot-ik", "sky-clock"]),
    // The owner's own layout of the movement work (tech/movement-inertia, 2026-11-15).
    section(NVF_FEATURES_TAB, Some("Advanced Movement"), &["advanced-movement"]),
    section(NVF_FEATURES_TAB, Some("Gameplay"), &["movement-inertia", "weapon-handling", "advanced-recoil"]),
    section(NVF_FEATURES_TAB, Some("Visual Only"), &["weapon-inertia", "animation-smoothing", "camera-motion"]),
    section(NVF_FEATURES_TAB, Some("World Interaction"), &["grass-fire"]),
    section("Debug", None, &["overlay", "hit-markers", "god-mode", "navmesh", "scope-3d"]),
];

/// The Controls tab's rows that are shown only while the input device is a controller
/// (`crate::bindings::InputDevice`) [design: #1953].
pub const CONTROLLER_KEYS: [&str; 4] = ["button-style", "aim-assist", "aim-slowdown", "aim-lock"];

/// Whether row `key` is shown only with a controller ([`CONTROLLER_KEYS`]).
pub fn controller_only(key: &str) -> bool {
    CONTROLLER_KEYS.contains(&key)
}

/// The table's faults: a key named twice, a tab not in [`TAB_ORDER`], a group outside the
/// Tweaks tab, a category without a heading on a tab with categories, a key in NVF Features
/// the owner did not name.
pub fn check(sections: &[Section]) -> Result<(), String> {
    let mut seen: Vec<&str> = Vec::new();
    for s in sections {
        if !TAB_ORDER.contains(&s.placement.tab) {
            return Err(format!("{}: not one of the tabs", s.placement.tab));
        }
        if s.placement.subgroup.is_some() && s.placement.tab != TWEAKS_TAB {
            return Err(format!("{}: a group is for the Tweaks tab only", s.placement.path()));
        }
        if s.keys.is_empty() {
            return Err(format!("{}: an empty category", s.placement.path()));
        }
        for k in s.keys {
            if seen.contains(k) {
                return Err(format!("{k}: placed twice"));
            }
            if s.placement.tab == NVF_FEATURES_TAB && !NVF_FEATURES_KEYS.contains(k) {
                return Err(format!("{k}: NVF Features holds only what the owner named there"));
            }
            if k.starts_with(CONTROL_PREFIX) && s.placement.tab != CONTROLS_TAB {
                return Err(format!("{k}: a Controls row outside the Controls tab"));
            }
            seen.push(k);
        }
    }
    for tab in TAB_ORDER {
        let of_tab: Vec<&Section> = sections.iter().filter(|s| s.placement.tab == tab).collect();
        if of_tab.len() > 1 && of_tab.iter().any(|s| s.placement.heading.is_none()) {
            return Err(format!("{tab}: every category of a tab with several needs a heading"));
        }
    }
    Ok(())
}

/// One line of a section: a registered row (its index in the registry) or a Controls row
/// (its name after [`CONTROL_PREFIX`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutItem {
    Setting(usize),
    Control(&'static str),
}

/// A section of the rows: its place and its lines in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedSection {
    pub placement: Placement,
    /// Its registered rows (indices in the registry), in order.
    pub rows: Vec<usize>,
    /// Its lines in order: the registered rows and the Controls rows among them.
    pub items: Vec<LayoutItem>,
}

/// A tab and its sections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTab {
    pub name: &'static str,
    pub sections: Vec<ResolvedSection>,
}

impl ResolvedTab {
    /// The Tweaks tab's groups, in the table's order (none on another tab).
    pub fn subgroups(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = Vec::new();
        for g in self.sections.iter().filter_map(|s| s.placement.subgroup) {
            if !out.contains(&g) {
                out.push(g);
            }
        }
        out
    }
}

/// One line of a tab as the table gives it: a category's heading, a registered row, a
/// Controls row, or the empty line after an open category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutEntry {
    Heading(Placement),
    Setting(usize),
    Control(&'static str),
    Gap,
}

impl From<LayoutItem> for LayoutEntry {
    fn from(i: LayoutItem) -> LayoutEntry {
        match i {
            LayoutItem::Setting(i) => LayoutEntry::Setting(i),
            LayoutItem::Control(c) => LayoutEntry::Control(c),
        }
    }
}

/// The table applied to the registered rows ([`resolve_layout`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedLayout {
    pub tabs: Vec<ResolvedTab>,
    /// Each registered row's place, by its index.
    placements: Vec<Placement>,
    /// The rows the table does not name.
    unnamed: usize,
}

/// The table applied to `list`, the registry in its order (module notes): the tabs (an
/// empty one too), each named section with the rows registered and its Controls rows (a
/// missing optional row left out, a section left with none dropped), then each tab's
/// unnamed rows, then the unnamed groups' tabs. Every registered row is in it once.
pub fn resolve_layout(list: &[Setting]) -> ResolvedLayout {
    let index: std::collections::HashMap<&str, usize> = list.iter().enumerate().map(|(i, s)| (s.key, i)).collect();
    let mut placements: Vec<Option<Placement>> = vec![None; list.len()];
    let mut tabs: Vec<ResolvedTab> = TAB_ORDER.iter().map(|&name| ResolvedTab { name, sections: Vec::new() }).collect();
    for s in SECTIONS {
        let mut items = Vec::new();
        for k in s.keys {
            if let Some(c) = k.strip_prefix(CONTROL_PREFIX) {
                items.push(LayoutItem::Control(c));
            } else if let Some(&i) = index.get(k).filter(|&&i| placements[i].is_none()) {
                placements[i] = Some(s.placement);
                items.push(LayoutItem::Setting(i));
            }
        }
        if items.is_empty() {
            continue;
        }
        let rows = items.iter().filter_map(|i| if let LayoutItem::Setting(i) = i { Some(*i) } else { None }).collect();
        if let Some(t) = tabs.iter_mut().find(|t| t.name == s.placement.tab) {
            t.sections.push(ResolvedSection { placement: s.placement, rows, items });
        }
    }
    // The unnamed rows, by tab, in registry order: one section after each tab's named ones.
    let mut unnamed: Vec<(usize, Vec<usize>)> = Vec::new();
    for (i, s) in list.iter().enumerate() {
        if placements[i].is_some() {
            continue;
        }
        placements[i] = Some(Placement::tab(s.group));
        let t = match tabs.iter().position(|t| t.name == s.group) {
            Some(t) => t,
            None => {
                tabs.push(ResolvedTab { name: s.group, sections: Vec::new() });
                tabs.len() - 1
            }
        };
        match unnamed.iter_mut().find(|(at, _)| *at == t) {
            Some((_, rows)) => rows.push(i),
            None => unnamed.push((t, vec![i])),
        }
    }
    let count = unnamed.iter().map(|(_, r)| r.len()).sum();
    for (t, rows) in unnamed {
        let placement = Placement::tab(tabs[t].name);
        let items = rows.iter().map(|&i| LayoutItem::Setting(i)).collect();
        tabs[t].sections.push(ResolvedSection { placement, rows, items });
    }
    ResolvedLayout { tabs, placements: placements.into_iter().map(|p| p.unwrap_or(Placement::tab(""))).collect(), unnamed: count }
}

impl ResolvedLayout {
    /// The tabs' names, in order.
    pub fn tab_names(&self) -> Vec<&'static str> {
        self.tabs.iter().map(|t| t.name).collect()
    }

    pub fn tab(&self, name: &str) -> Option<&ResolvedTab> {
        self.tabs.iter().find(|t| t.name == name)
    }

    /// Registered row `i`'s place.
    pub fn placement_of(&self, i: usize) -> Option<Placement> {
        self.placements.get(i).copied()
    }

    /// The place of the Controls row named `name` (after [`CONTROL_PREFIX`]).
    pub fn control_placement(&self, name: &str) -> Option<Placement> {
        self.tabs.iter().flat_map(|t| &t.sections).find(|s| s.items.iter().any(|i| matches!(i, LayoutItem::Control(c) if *c == name))).map(|s| s.placement)
    }

    /// How many registered rows the table does not name.
    pub fn unnamed(&self) -> usize {
        self.unnamed
    }

    /// Every registered row in the page's order: tab by tab, section by section (what
    /// search and the Changed list go by).
    pub fn ordered_settings(&self) -> impl Iterator<Item = usize> + '_ {
        self.tabs.iter().flat_map(|t| t.sections.iter().flat_map(|s| s.rows.iter().copied()))
    }

    /// The section at `p`.
    pub fn section(&self, p: &Placement) -> Option<&ResolvedSection> {
        self.tab(p.tab)?.sections.iter().find(|s| s.placement == *p)
    }

    /// Tab `name`'s lines (of its group `subgroup` on the Tweaks tab; `None`: every
    /// section): each category with a line `visible` says is shown, its heading (when it has
    /// one) and, while `open` says it is open, those lines; after each open category with
    /// another after it, an empty line [design: the overhaul]. A category whose lines are all
    /// hidden is left out, heading too; a hidden line keeps its place for when it shows.
    pub fn tab_entries(&self, name: &str, subgroup: Option<&str>, visible: impl Fn(LayoutItem) -> bool, open: impl Fn(&Placement) -> bool) -> Vec<LayoutEntry> {
        let mut out = Vec::new();
        let Some(t) = self.tab(name) else { return out };
        let mut after_open = false;
        for s in t.sections.iter().filter(|s| subgroup.is_none() || s.placement.subgroup == subgroup) {
            let items: Vec<LayoutItem> = s.items.iter().copied().filter(|&i| visible(i)).collect();
            if items.is_empty() {
                continue;
            }
            if after_open {
                out.push(LayoutEntry::Gap);
            }
            let headed = s.placement.heading.is_some();
            let shows = !headed || open(&s.placement);
            if headed {
                out.push(LayoutEntry::Heading(s.placement));
            }
            if shows {
                out.extend(items.into_iter().map(LayoutEntry::from));
            }
            after_open = shows;
        }
        out
    }

    /// How many of the category at `p`'s lines `visible` says are shown (what its heading
    /// counts while closed).
    pub fn shown_count(&self, p: &Placement, visible: impl Fn(LayoutItem) -> bool) -> usize {
        self.section(p).map_or(0, |s| s.items.iter().filter(|&&i| visible(i)).count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    fn row(key: &'static str, group: &'static str) -> Setting {
        Setting { key, group, label: key, note: "", switches: "", get: |_| false, set: |_, _| {}, slider: None }
    }

    /// The registry as `Settings::add` orders it.
    fn registry(rows: &[(&'static str, &'static str)]) -> Vec<Setting> {
        let mut s = Settings::default();
        for (k, g) in rows {
            s.add(row(k, g));
        }
        s.list().to_vec()
    }

    /// A tab's lines, every category open, every line shown: `[Heading]`, keys, `@control`,
    /// and "" for an empty line.
    fn keys(list: &[Setting], l: &ResolvedLayout, tab: &str) -> Vec<String> {
        lines(list, l, tab, |_| true)
    }

    fn lines(list: &[Setting], l: &ResolvedLayout, tab: &str, open: impl Fn(&Placement) -> bool) -> Vec<String> {
        l.tab_entries(tab, None, |_| true, open)
            .into_iter()
            .map(|e| match e {
                LayoutEntry::Heading(p) => format!("[{}]", p.heading.unwrap_or("")),
                LayoutEntry::Setting(i) => list[i].key.to_string(),
                LayoutEntry::Control(c) => format!("@{c}"),
                LayoutEntry::Gap => String::new(),
            })
            .collect()
    }

    /// Every row the registry can hold today (upstream main af16a793f: #1966's quality and
    /// enhanced rows, #2014's counter rows, the VATS rows, #2198's upscaler rows, #2169's FSR
    /// library row, #2242's upscaling counter row, #2317's Generated frames row, the field of
    /// view rows, the menu scale and Physics rate rows, #2662's grass fires and #2550's
    /// terrain height blending), by the group it registers with.
    const KNOWN: [(&str, &str); 114] = [
        ("detached-camera", "Gameplay"),
        ("menu-pause", "Gameplay"),
        ("dialogue-goodbye-hint", "Gameplay"),
        ("reload-slows", "Gameplay"),
        ("grass-fire", "Gameplay"),
        ("music", "Gameplay"),
        ("voice", "Gameplay"),
        ("dialogue-look", "Gameplay"),
        ("dialogue-recentre", "Gameplay"),
        ("dialogue-zoom", "Gameplay"),
        ("dynamic-crosshair", "Gameplay"),
        ("crosshair-style", "Gameplay"),
        ("sneak-gauges", "Gameplay"),
        ("UsePipBoy", "Gameplay"),
        ("kill-camera", "Gameplay"),
        ("vats-style", "Gameplay"),
        ("vats-slowdown", "Gameplay"),
        ("difficulty", "Gameplay"),
        ("hardcore", "Gameplay"),
        ("lip-sync", "Gameplay"),
        ("true-iron-sights", "Gameplay"),
        ("scope-3d", "Gameplay"),
        ("button-style", "Controller"),
        ("aim-assist", "Controller"),
        ("aim-slowdown", "Controller"),
        ("aim-lock", "Controller"),
        ("autosave-travel", "Saving"),
        ("autosave-fast-travel", "Saving"),
        ("autosave-wait", "Saving"),
        ("autosave-sleep", "Saving"),
        ("autosave-scripts", "Saving"),
        ("hud-colour", "Interface"),
        ("hud-red", "Interface"),
        ("hud-green", "Interface"),
        ("hud-blue", "Interface"),
        ("pipboy-colour", "Interface"),
        ("pipboy-red", "Interface"),
        ("pipboy-green", "Interface"),
        ("pipboy-blue", "Interface"),
        ("language", "Interface"),
        ("head-tracking", "World"),
        ("foot-ik", "World"),
        ("sky-clock", "World"),
        ("resolution", "Display"),
        ("borderless", "Display"),
        ("monitor", "Display"),
        ("first-person-body", "Display"),
        ("weapon-sway", "Display"),
        ("head-bob", "Display"),
        ("weapon-lower", "Display"),
        ("sun-shadows", "Display"),
        ("grass", "Display"),
        ("distant-lod", "Display"),
        ("grade", "Display"),
        ("field-of-view", "Display"),
        ("viewmodel-field-of-view", "Display"),
        ("ui-scale", "Display"),
        ("menu-scale", "Display"),
        ("fps-counter", "Display"),
        ("fps-counter-fps", "Display"),
        ("fps-counter-low", "Display"),
        ("fps-counter-graph", "Display"),
        ("fps-counter-split", "Display"),
        ("fps-counter-cpu", "Display"),
        ("fps-counter-gpu", "Display"),
        ("fps-counter-vram", "Display"),
        ("fps-counter-ram", "Display"),
        ("fps-counter-loaded", "Display"),
        ("fps-counter-upscaling", "Display"),
        ("display-sync", "Display"),
        ("frame-cap", "Display"),
        ("physics-rate", "Display"),
        ("object-distance", "Display"),
        ("actor-distance", "Display"),
        ("item-distance", "Display"),
        ("shadow-distance", "Display"),
        ("grass-distance", "Display"),
        ("upscaler", "Graphics"),
        ("upscaler-quality", "Graphics"),
        ("dlss-model", "Graphics"),
        ("fsr-library", "Graphics"),
        ("frame-generation", "Graphics"),
        ("frame-generation-frames", "Graphics"),
        ("anti-aliasing", "Graphics"),
        ("latency-reduction", "Graphics"),
        ("shadow-filter", "Graphics"),
        ("ambient-occlusion", "Graphics"),
        ("fine-shadows", "Graphics"),
        ("bloom", "Graphics"),
        ("glow", "Graphics"),
        ("eye-adaptation", "Graphics"),
        ("neon-glow", "Graphics"),
        ("sharpen", "Graphics"),
        ("anisotropic", "Graphics"),
        ("terrain-height-blend", "Graphics"),
        ("light-shafts", "Graphics"),
        ("water-reflections", "Graphics"),
        ("water-mirror", "Graphics"),
        ("enhanced", "Graphics"),
        ("quality", "Graphics"),
        ("volume-master", "Sound"),
        ("volume-music", "Sound"),
        ("volume-footstep", "Sound"),
        ("volume-voice", "Sound"),
        ("volume-effects", "Sound"),
        ("volume-radio", "Sound"),
        ("reverb", "Sound"),
        ("voice-reverb", "Sound"),
        ("overlay", "Debug"),
        ("hit-markers", "Debug"),
        ("god-mode", "Debug"),
        ("navmesh", "Debug"),
        ("send-reports", "Reports"),
        ("reports-delete", "Reports"),
    ];

    /// The rows the table places ahead of the change that registers them (the movement
    /// rows of tech/movement-inertia).
    const AHEAD: [&str; 7] = ["advanced-movement", "movement-inertia", "weapon-handling", "advanced-recoil", "weapon-inertia", "animation-smoothing", "camera-motion"];

    #[test]
    fn the_table_names_each_key_once_in_one_of_the_tabs() {
        assert_eq!(check(SECTIONS), Ok(()));
        let twice = [section("Gameplay", Some("A"), &["a"]), section("Debug", None, &["a"])];
        assert!(check(&twice).unwrap_err().contains("placed twice"));
        let odd = [section("Sound", None, &["a"])];
        assert!(check(&odd).is_err());
        let unasked = [section(NVF_FEATURES_TAB, Some("New"), &["a-big-change"])];
        assert!(check(&unasked).unwrap_err().contains("owner named"), "NVF Features only by the owner's word");
        let control = [section("Gameplay", Some("A"), &["controls:device"])];
        assert!(check(&control).is_err());
        let headless = [section("Gameplay", Some("A"), &["a"]), section("Gameplay", None, &["b"])];
        assert!(check(&headless).is_err());
        // Every row there is has its place in the table; the table names no other but the
        // Controls tab's own rows and the rows placed ahead.
        let named: Vec<&str> = SECTIONS.iter().flat_map(|s| s.keys.iter().copied()).filter(|k| !k.starts_with(CONTROL_PREFIX)).collect();
        for (k, _) in KNOWN {
            assert!(named.contains(&k), "{k} has no place");
        }
        for k in &named {
            assert!(KNOWN.iter().any(|(n, _)| n == k) || AHEAD.contains(k), "{k} is placed but no row has it");
        }
    }

    #[test]
    fn the_tabs_come_first_with_every_row_once_in_the_tables_order() {
        let list = registry(&KNOWN);
        let l = resolve_layout(&list);
        assert_eq!(l.tab_names(), TAB_ORDER, "no group is a tab of its own: every row is named");
        assert_eq!(l.unnamed(), 0);
        let mut all: Vec<usize> = l.ordered_settings().collect();
        assert_eq!(all.len(), list.len());
        all.sort_unstable();
        assert_eq!(all, (0..list.len()).collect::<Vec<_>>(), "each registered row once");
        assert_eq!(keys(&list, &l, "Gameplay")[..10], ["[Game]", "difficulty", "hardcore", "reload-slows", "UsePipBoy", "", "[V.A.T.S.]", "vats-style", "vats-slowdown", "kill-camera"]);
        assert_eq!(keys(&list, &l, "Audio"), ["[Volume]", "volume-master", "volume-music", "volume-footstep", "volume-voice", "volume-effects", "volume-radio", "", "[Sound]", "music", "voice", "reverb", "voice-reverb"]);
        assert_eq!(keys(&list, &l, "Controls")[..6], ["[Input]", "@device", "button-style", "@automatic-input", "@rumble", ""]);
        assert_eq!(keys(&list, &l, NVF_FEATURES_TAB), ["[World Interaction]", "grass-fire"], "the movement rows are not registered on main");
        assert_eq!(keys(&list, &l, "Debug"), ["overlay", "hit-markers", "god-mode", "navmesh", "scope-3d"]);
        assert!(keys(&list, &l, "Tweaks").is_empty(), "an empty tab is still a tab");
        let at = |k: &str| l.placement_of(list.iter().position(|s| s.key == k).unwrap()).unwrap().path();
        assert_eq!(at("menu-pause"), "Gameplay > Dialogue");
        assert_eq!(at("hud-red"), "Display > Colours");
        assert_eq!(at("upscaler"), "Display > Upscaling and Anti-Aliasing");
        assert_eq!(at("sky-clock"), "Graphics > World and Animation");
        assert_eq!(at("grass-fire"), "NVF Features > World Interaction");
        assert_eq!(at("button-style"), "Controls > Input");
        assert_eq!(l.control_placement("bind:Jump").unwrap().path(), "Controls > Movement");
        // The registry itself is left as it was: its order and each row's group.
        let again = registry(&KNOWN);
        assert!(list.iter().zip(&again).all(|(a, b)| a.key == b.key && a.group == b.group && a.label == b.label && a.note == b.note));
    }

    #[test]
    fn a_closed_category_keeps_its_heading_and_an_empty_line_follows_only_an_open_one() {
        let list = registry(&KNOWN);
        let l = resolve_layout(&list);
        let closed = |p: &Placement| p.heading != Some("V.A.T.S.") && p.heading != Some("Camera");
        let g = lines(&list, &l, "Gameplay", closed);
        assert_eq!(g[..9], ["[Game]", "difficulty", "hardcore", "reload-slows", "UsePipBoy", "", "[V.A.T.S.]", "[Camera]", "[Dialogue]"]);
        let p = l.placement_of(list.iter().position(|s| s.key == "vats-style").unwrap()).unwrap();
        assert_eq!(l.shown_count(&p, |_| true), 3);
        assert_eq!(l.shown_count(&p, |i| i != LayoutItem::Setting(list.iter().position(|s| s.key == "vats-slowdown").unwrap())), 2);
    }

    #[test]
    fn a_missing_row_and_an_empty_category_are_left_out() {
        // No lip sync, no player body; no head tracking, foot IK or sky clock at all.
        let list = registry(&[("difficulty", "Gameplay"), ("menu-pause", "Gameplay"), ("head-bob", "Display"), ("god-mode", "Debug")]);
        let l = resolve_layout(&list);
        assert_eq!(keys(&list, &l, "Gameplay"), ["[Game]", "difficulty", "", "[Camera]", "head-bob", "", "[Dialogue]", "menu-pause"]);
        assert!(keys(&list, &l, "Graphics").is_empty());
        // A hidden row hides its category when it is the only one, and keeps its place.
        let hidden = list.iter().position(|s| s.key == "menu-pause").unwrap();
        let shown = l.tab_entries("Gameplay", None, |i| i != LayoutItem::Setting(hidden), |_| true);
        assert!(!shown.iter().any(|e| matches!(e, LayoutEntry::Heading(p) if p.heading == Some("Dialogue"))));
        assert_ne!(shown.last(), Some(&LayoutEntry::Gap), "no empty line at the end");
        assert_eq!(l.placement_of(hidden).unwrap().path(), "Gameplay > Dialogue");
    }

    #[test]
    fn an_unnamed_row_stays_under_its_own_group() {
        let list = registry(&[("difficulty", "Gameplay"), ("new-switch", "Gameplay"), ("odd-sound", "Sound"), ("x1", "Mods"), ("volume-master", "Sound"), ("x2", "Extras"), ("x3", "Mods")]);
        let l = resolve_layout(&list);
        // After Debug, in the order the groups first appear in the registry; Sound is not Audio.
        assert_eq!(l.tab_names()[8..], ["Sound", "Mods", "Extras"]);
        assert_eq!(keys(&list, &l, "Gameplay"), ["[Game]", "difficulty", "", "new-switch"]);
        assert_eq!(keys(&list, &l, "Sound"), ["odd-sound"]);
        assert_eq!(keys(&list, &l, "Audio"), ["[Volume]", "volume-master"]);
        assert_eq!(keys(&list, &l, "Mods"), ["x1", "x3"]);
        let new = list.iter().position(|s| s.key == "new-switch").unwrap();
        assert_eq!(l.placement_of(new), Some(Placement::tab("Gameplay")));
        assert_eq!(l.unnamed(), 5);
    }

    #[test]
    fn hundreds_of_unnamed_tweaks_keep_the_registrys_order() {
        // A tweak the table does not name yet: under Tweaks, in the order registered.
        let names: Vec<&'static str> = (0..650)
            .map(|i| {
                let n: &'static str = Box::leak(format!("tweak-x{i:03}").into_boxed_str());
                n
            })
            .collect();
        let mut rows: Vec<(&'static str, &'static str)> = vec![("difficulty", "Gameplay")];
        rows.extend(names.iter().map(|n| (*n, "Tweaks")));
        rows.push(("god-mode", "Debug"));
        let list = registry(&rows);
        let l = resolve_layout(&list);
        assert_eq!(l.tab_names(), TAB_ORDER);
        let shown = keys(&list, &l, "Tweaks");
        assert_eq!(shown.len(), 650, "no heading: all in one run");
        assert!(shown.iter().zip(&names).all(|(a, b)| a == b), "in the order registered");
        assert_eq!(l.ordered_settings().count(), list.len());
    }
}
