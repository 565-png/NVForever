//! The settings page's menu as the launcher shows it: the game's own [design: owner
//! 2026-11-16: the launcher follows the game's menus, so it never names a tab or category
//! the game does not have]. Two sources, both the game's:
//!
//! - **The game's menu file** (`settings-menu.json` beside `NVForever.ini`,
//!   `nvf_settings_layout::menu`): the page as the game laid it out at its last start, every
//!   tab, category and row with its label, note and kind (switch, slider and its steps' words,
//!   list and its choices). Used while its version is the launcher's ([`Catalog::from_menu`]).
//! - **Built in** ([`Catalog::built_in`]), before the game has written one (a first start, a
//!   game older than the file, an update not started yet): the tabs, categories and row order
//!   from the game's own table (`nvf_settings_layout::SECTIONS`, the crate the game's pause
//!   menu draws from), the labels and notes from `docs/settings-reference.md` (a test in the
//!   game keeps it the same as the table: `settings_layout.rs`,
//!   `the_reference_has_every_row_where_the_page_shows_it`). What kind of row a key is the
//!   launcher then goes by the value the INI holds ([`Kind::of_value`]).

use std::collections::HashMap;
use std::path::Path;

use nvf_settings_layout::menu::{self, Menu};
use nvf_settings_layout::{CONTROL_PREFIX, SECTIONS, TAB_ORDER};

const REFERENCE: &str = include_str!("../../../docs/settings-reference.md");

/// The heading after which the reference lists the Controls tab's own rows (kept in
/// `[Controls]`, not `[Settings]`): the launcher stops there.
const OWN_ROWS: &str = "the tab's own rows";

/// One row of the settings page.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub key: String,
    pub label: String,
    /// Its tab's name in the game's table (`TAB_ORDER`).
    pub tab: String,
    /// Its category's heading as shown (the Tweaks tab's: "Group › Heading").
    pub heading: Option<String>,
    pub note: String,
    /// How it changes, from the game's menu file; `None` (built in): by the INI's value.
    pub kind: Option<menu::Kind>,
    /// Built in, with no words in the reference (a Tweaks row): shown only while the INI
    /// holds it, labelled by its key.
    pub unlisted: bool,
    /// Shown only while this holds, greyed while one of `needs` does not: the game's rules
    /// (`nvf_settings_layout::shown_with`, `needs`), the same in both sources.
    pub shown_with: Option<menu::Rule>,
    pub needs: Vec<menu::Rule>,
}

/// The page: its tabs (the table's name, the words shown) and rows, in the page's order.
#[derive(Clone, Debug, PartialEq)]
pub struct Catalog {
    pub tabs: Vec<(String, String)>,
    pub rows: Vec<Row>,
    /// From the game's menu file (not built in).
    pub from_game: bool,
}

impl Catalog {
    /// The game's menu beside `ini` when it is this launcher's version, else the built-in one.
    pub fn load(ini: Option<&Path>) -> Catalog {
        ini.and_then(|p| std::fs::read_to_string(p.with_file_name(menu::MENU_FILE)).ok())
            .and_then(|t| Menu::parse(&t))
            .and_then(|m| Catalog::from_menu(&m, env!("CARGO_PKG_VERSION")))
            .unwrap_or_else(Catalog::built_in)
    }

    /// The game's menu `m`, when it is version `version`'s.
    pub fn from_menu(m: &Menu, version: &str) -> Option<Catalog> {
        // Not this version's, or with no row (a broken write): the built-in copy.
        if m.version != version || m.rows().next().is_none() {
            return None;
        }
        let mut rows = Vec::new();
        for t in &m.tabs {
            for c in &t.categories {
                let heading = match (&c.group, &c.heading) {
                    (Some(g), Some(h)) => Some(format!("{g} › {h}")),
                    (g, h) => g.clone().or_else(|| h.clone()),
                };
                for r in &c.rows {
                    // The file's rules (their reasons in the game's language); a file written
                    // before it had them: the shared table's, the same game version's.
                    let english = |w: &'static str| w.to_string();
                    let shown_with = r.shown_with.clone().or_else(|| nvf_settings_layout::shown_with(&r.key).map(|x| menu::Rule::of(x, english)));
                    let needs = if r.needs.is_empty() { nvf_settings_layout::needs(&r.key).into_iter().map(|x| menu::Rule::of(x, english)).collect() } else { r.needs.clone() };
                    rows.push(Row {
                        key: r.key.clone(),
                        label: r.label.clone(),
                        tab: t.name.clone(),
                        heading: heading.clone(),
                        note: r.note.clone(),
                        kind: Some(r.kind.clone()),
                        unlisted: false,
                        shown_with,
                        needs,
                    });
                }
            }
        }
        let tabs = m.tabs.iter().map(|t| (t.name.clone(), t.label.clone())).collect();
        Some(Catalog { tabs, rows, from_game: true })
    }

    /// The game's table with the reference's words (module notes).
    pub fn built_in() -> Catalog {
        let words: HashMap<String, Row> = parse(REFERENCE).into_iter().map(|r| (r.key.clone(), r)).collect();
        let mut rows = Vec::new();
        for s in SECTIONS {
            let p = s.placement;
            let heading = match (p.subgroup, p.heading) {
                (Some(g), Some(h)) => Some(format!("{g} › {h}")),
                (g, h) => g.or(h).map(str::to_string),
            };
            for k in s.keys.iter().filter(|k| !k.starts_with(CONTROL_PREFIX)) {
                let (label, note, unlisted) = match words.get(*k) {
                    Some(w) => (w.label.clone(), w.note.clone(), false),
                    None => (k.to_string(), String::new(), true),
                };
                let english = |w: &'static str| w.to_string();
                let shown_with = nvf_settings_layout::shown_with(k).map(|r| menu::Rule::of(r, english));
                let needs = nvf_settings_layout::needs(k).into_iter().map(|r| menu::Rule::of(r, english)).collect();
                rows.push(Row { key: k.to_string(), label, tab: p.tab.to_string(), heading: heading.clone(), note, kind: None, unlisted, shown_with, needs });
            }
        }
        let tabs = TAB_ORDER.iter().filter(|t| rows.iter().any(|r| r.tab == **t)).map(|t| (t.to_string(), t.to_string())).collect();
        Catalog { tabs, rows, from_game: false }
    }

    /// The tabs' names, in order.
    pub fn tab_names(&self) -> Vec<String> {
        self.tabs.iter().map(|(n, _)| n.clone()).collect()
    }
}

/// The rows of `text` (the reference's Markdown): key, label, tab, heading and note.
pub fn parse(text: &str) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();
    let (mut tab, mut heading) = (String::new(), None::<String>);
    let mut in_note = false;
    for line in text.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            if h.contains(OWN_ROWS) {
                break;
            }
            let mut parts = h.split('›').map(str::trim);
            tab = parts.next().unwrap_or_default().to_string();
            heading = parts.next().map(str::to_string);
            in_note = false;
        } else if let Some(h) = line.strip_prefix("### `") {
            in_note = false;
            if let Some((key, rest)) = h.split_once('`') {
                let label = rest.trim_start_matches(':').trim().to_string();
                out.push(Row { key: key.to_string(), label, tab: tab.clone(), heading: heading.clone(), note: String::new(), kind: None, unlisted: false, shown_with: None, needs: Vec::new() });
            }
        } else if let Some(n) = line.strip_prefix("- On the page:") {
            if let Some(r) = out.last_mut() {
                r.note = n.trim().to_string();
                in_note = true;
            }
        } else if in_note && line.starts_with("  ") && !line.trim_start().starts_with("- ") {
            if let Some(r) = out.last_mut() {
                r.note.push(' ');
                r.note.push_str(line.trim());
            }
        } else {
            in_note = false;
        }
    }
    out
}

/// How a row is changed, from the text its INI line holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `key=1` / `key=0`.
    Switch,
    /// A part of a full amount, `key=0.50`.
    Slider,
    /// Anything else: a list's choice by its own text (`resolution=1600x900;desktop`).
    Text,
}

impl Kind {
    pub fn of_value(v: &str) -> Kind {
        match v.trim() {
            "0" | "1" => Kind::Switch,
            t if t.contains('.') && t.parse::<f32>().is_ok() => Kind::Slider,
            _ => Kind::Text,
        }
    }
}

/// Sliders whose steps the page shows as words, not a percentage: the words for each step,
/// first to last (the step `i` of `n` is kept as `i / (n - 1)`, `Slider::choice` in
/// engine/nvf-play/src/settings.rs). Words are NVForever's own (the page's).
pub fn step_words() -> HashMap<&'static str, &'static [&'static str]> {
    HashMap::from([("difficulty", &["Very Easy", "Easy", "Normal", "Hard", "Very Hard"][..])])
}

/// List rows whose choices are fixed, by their names as the INI keeps them (each list's
/// `name()` in the engine; a test checks the engine still has every name). Other lists
/// (the resolution, the monitor) depend on the machine and stay text.
pub fn list_choices() -> HashMap<&'static str, &'static [&'static str]> {
    HashMap::from([("kill-camera", &["Last enemy", "Every kill", "Off"][..]), ("vats-style", &["Classic", "Mark and Fire"][..])])
}

#[cfg(test)]
mod tests {
    use super::*;
    use nvf_settings_layout::TWEAKS_TAB;

    #[test]
    fn parses_rows_tabs_headings_and_notes() {
        let t = "# S\n\n## Gameplay › Game\n\n### `difficulty`: Difficulty\n\n- On the page: How hard the\n  fights are.\n- In full: more\n  words.\n- Registered in x\n\n## Audio\n\n### `music`: Music\n\n- On the page: Plays it.\n\n## Controls › the tab's own rows\n\n### `x`: X\n";
        let r = parse(t);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], Row { key: "difficulty".into(), label: "Difficulty".into(), tab: "Gameplay".into(), heading: Some("Game".into()), note: "How hard the fights are.".into(), kind: None, unlisted: false, shown_with: None, needs: vec![] });
        assert_eq!((r[1].tab.as_str(), r[1].heading.as_deref(), r[1].note.as_str()), ("Audio", None, "Plays it."));
    }

    #[test]
    fn built_in_the_games_tabs_categories_and_order_with_the_references_words() {
        let c = Catalog::built_in();
        assert!(!c.from_game);
        let names = c.tab_names();
        assert_eq!(names, TAB_ORDER.iter().filter(|t| names.contains(&t.to_string())).map(|t| t.to_string()).collect::<Vec<_>>(), "the game's tabs in its order");
        assert!(names.starts_with(&["Gameplay".to_string(), "Display".to_string()]));
        // Every row where the game's table places it, in its order; no Controls row of its own.
        let placed: Vec<(&str, &str)> = SECTIONS.iter().flat_map(|s| s.keys.iter().filter(|k| !k.starts_with(CONTROL_PREFIX)).map(move |k| (*k, s.placement.tab))).collect();
        assert_eq!(c.rows.iter().map(|r| (r.key.as_str(), r.tab.as_str())).collect::<Vec<_>>(), placed);
        let at = |k: &str| c.rows.iter().find(|r| r.key == k).unwrap_or_else(|| panic!("{k} missing"));
        assert_eq!((at("menu-pause").tab.as_str(), at("menu-pause").heading.as_deref()), ("Gameplay", Some("Dialogue")));
        for key in ["difficulty", "menu-pause", "resolution", "borderless"] {
            let row = at(key);
            assert!(!row.unlisted && !row.note.is_empty() && !row.label.is_empty(), "{row:?}");
        }
        // The Tweaks tab's rows: the game's groups and categories, labelled by key until the
        // game writes its menu.
        assert_eq!(at("dlss-model").shown_with.as_ref().map(|r| r.parent.as_str()), Some("upscaler"), "the game's rules built in too");
        assert_eq!(at("aim-lock").shown_with.as_ref().map(|r| r.choices.clone()), Some(vec!["Controller".to_string()]));
        if let Some(t) = c.rows.iter().find(|r| r.tab == TWEAKS_TAB) {
            assert!(t.unlisted && t.heading.as_deref().is_some_and(|h| h.contains(" › ")), "{t:?}");
        }
    }

    /// The launcher never shows a row the game's table does not place, nor in another tab or
    /// category than the game's [design: owner 2026-11-16].
    #[test]
    fn every_reference_row_is_where_the_game_places_it() {
        for r in parse(REFERENCE) {
            let s = SECTIONS.iter().find(|s| s.keys.contains(&r.key.as_str())).unwrap_or_else(|| panic!("{}: in the reference, not in the game's menu", r.key));
            assert_eq!((s.placement.tab, s.placement.heading), (r.tab.as_str(), r.heading.as_deref()), "{}: the reference's place is not the game's", r.key);
        }
    }

    fn game_menu(version: &str) -> Menu {
        use menu::{Category, Kind as K, Row as R, Tab};
        let row = |key: &str, kind: K| R { key: key.into(), label: key.to_uppercase(), note: "n".into(), kind, shown_with: None, needs: vec![] };
        Menu {
            version: version.into(),
            tabs: vec![
                Tab { name: "Gameplay".into(), label: "Gameplay".into(), categories: vec![Category { group: None, heading: Some("Game".into()), rows: vec![row("hardcore", K::Switch)] }] },
                Tab {
                    name: TWEAKS_TAB.into(),
                    label: "Tweaks".into(),
                    categories: vec![Category { group: Some("Combat".into()), heading: Some("In Combat".into()), rows: vec![row("tweak-no-combat-music", K::Switch)] }],
                },
            ],
        }
    }

    #[test]
    fn the_games_menu_file_when_it_is_this_version() {
        let m = game_menu(env!("CARGO_PKG_VERSION"));
        let c = Catalog::from_menu(&m, env!("CARGO_PKG_VERSION")).unwrap();
        assert!(c.from_game);
        assert_eq!(c.tab_names(), ["Gameplay", TWEAKS_TAB]);
        assert_eq!(c.rows[1].heading.as_deref(), Some("Combat › In Combat"));
        assert_eq!((c.rows[1].label.as_str(), c.rows[1].kind.clone()), ("TWEAK-NO-COMBAT-MUSIC", Some(menu::Kind::Switch)));
        // The game's rules: from the file, else (a file without them) the shared table.
        let mut with_rules = game_menu(env!("CARGO_PKG_VERSION"));
        with_rules.tabs[0].categories[0].rows.push(menu::Row { key: "shadow-distance".into(), label: "S".into(), note: "n".into(), kind: menu::Kind::Switch, shown_with: None, needs: vec![] });
        with_rules.tabs[0].categories[0].rows.push(menu::Row {
            key: "vats-slowdown".into(),
            label: "V".into(),
            note: "n".into(),
            kind: menu::Kind::Switch,
            shown_with: None,
            needs: vec![menu::Rule { parent: "vats-style".into(), choices: vec!["Classic".into()], why: "Nur klassisch.".into() }],
        });
        let w = Catalog::from_menu(&with_rules, env!("CARGO_PKG_VERSION")).unwrap();
        let at = |k: &str| w.rows.iter().find(|r| r.key == k).unwrap().clone();
        assert_eq!(at("shadow-distance").needs.iter().map(|r| (r.parent.as_str(), r.why.as_str())).collect::<Vec<_>>(), [("sun-shadows", "Needs Sun Shadows.")]);
        assert_eq!(at("vats-slowdown").needs[0].why, "Nur klassisch.", "the file's words win");
        assert_eq!(at("hardcore").needs, vec![]);
        assert_eq!(Catalog::from_menu(&m, "0.0.1"), None, "another version's menu: built in");
        let empty = Menu { version: env!("CARGO_PKG_VERSION").into(), tabs: vec![] };
        assert_eq!(Catalog::from_menu(&empty, env!("CARGO_PKG_VERSION")), None, "no row: built in, not an empty page");
        let dir = std::env::temp_dir().join(format!("nvf-launcher-menu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ini = dir.join("NVForever.ini");
        assert!(!Catalog::load(Some(&ini)).from_game, "no file: built in");
        std::fs::write(dir.join(menu::MENU_FILE), m.to_text()).unwrap();
        assert_eq!(Catalog::load(Some(&ini)), c);
        std::fs::write(dir.join(menu::MENU_FILE), game_menu("0.0.1").to_text()).unwrap();
        assert!(!Catalog::load(Some(&ini)).from_game, "an older game's: built in");
        std::fs::write(dir.join(menu::MENU_FILE), "{").unwrap();
        assert!(!Catalog::load(Some(&ini)).from_game, "broken: built in");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_choices_match_the_engines_names() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../engine/nvf-play/src/");
        for (key, file) in [("kill-camera", "kill_cam.rs"), ("vats-style", "vats_classic.rs")] {
            let src = std::fs::read_to_string(format!("{root}{file}")).unwrap();
            for name in list_choices()[key] {
                assert!(src.contains(&format!("=> \"{name}\"")), "{key}: {name} is no longer a choice in {file}");
            }
        }
    }

    #[test]
    fn kinds_from_values() {
        assert_eq!(Kind::of_value("1"), Kind::Switch);
        assert_eq!(Kind::of_value("0.50"), Kind::Slider);
        assert_eq!(Kind::of_value("1920x1080;desktop"), Kind::Text);
        assert_eq!(Kind::of_value("2"), Kind::Text);
    }
}
