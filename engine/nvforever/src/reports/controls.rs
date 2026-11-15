//! [design, #2403] Read-only controls-lock evidence for reports, registered only when
//! reports are enabled (`reports/mod.rs`). Runtime fields: `fnv_game::game::Game`;
//! engine screen state: `nvf_play::{menu,pause,pipboy,wait}`.

use super::anomaly::{self, LockProgress};
use super::Collector;
use bevy::ecs::system::SystemParam;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::MouseButtonInput;
use bevy::input::ButtonState;
use bevy::prelude::*;
use fnv_game::game::Game;
use serde_json::{json, Value};
use std::hash::{Hash, Hasher};

#[derive(SystemParam)]
pub(super) struct Presses<'w, 's> {
    keyboard: MessageReader<'w, 's, KeyboardInput>,
    mouse: MessageReader<'w, 's, MouseButtonInput>,
    pads: Query<'w, 's, &'static Gamepad>,
}

impl Presses<'_, '_> {
    fn count(&mut self) -> usize {
        // [design / file refs] Raw keyboard/mouse messages survive UI consumption;
        // pad.rs's synthesized menu keys are not messages and must not count again.
        self.keyboard.read().filter(key_press).count()
            + self.mouse.read().filter(|e| e.state == ButtonState::Pressed).count()
            + self.pads.iter().map(|p| p.get_just_pressed().count()).sum::<usize>()
    }
}

fn key_press(e: &&KeyboardInput) -> bool {
    e.state == ButtonState::Pressed && !e.repeat
}

#[derive(SystemParam)]
pub(super) struct Menus<'w> {
    state: Option<Res<'w, State<nvf_play::PlayState>>>,
    paused: Option<Res<'w, nvf_play::pause::Paused>>,
    page: Option<Res<'w, nvf_play::pause::Page>>,
    pause_view: Option<Res<'w, nvf_play::pause::PauseView>>,
    settings_tab: Option<Res<'w, nvf_play::pause::SettingsTabStore>>,
    pipboy: Option<Res<'w, nvf_play::pipboy::PipBoy>>,
    data: Option<Res<'w, nvf_play::pipboy::data::DataState>>,
    stats: Option<Res<'w, nvf_play::pipboy::stats::StatsView>>,
    repair: Option<Res<'w, nvf_play::pipboy::repair::PipRepair>>,
    item_mod: Option<Res<'w, nvf_play::pipboy::item_mod::PipItemMod>>,
    active: Option<Res<'w, nvf_play::menu::ActiveMenu>>,
    wait: Option<Res<'w, nvf_play::wait::WaitMenu>>,
    terminal: Option<Res<'w, nvf_play::terminal::TerminalScreen>>,
    levelup: Option<Res<'w, nvf_play::levelup::LevelUpMenu>>,
    dialogue: Option<Res<'w, nvf_play::dialogue::DialogueScreen>>,
}

fn hash_debug(value: &impl std::fmt::Debug, hash: &mut impl Hasher) {
    format!("{value:?}").hash(hash);
}

fn cut(label: &str) -> String {
    label.chars().take(anomaly::CONTROLS_LOCKED_LABEL_CHARS).collect()
}

fn menu_name(code: u16) -> String {
    // [design / file refs] Diagnostic labels for the existing `game::menus` codes.
    use fnv_game::game::menus;
    match code {
        menus::MESSAGE => "MessageMenu".into(),
        menus::INVENTORY => "InventoryMenu".into(),
        menus::STATS => "StatsMenu".into(),
        menus::START => "StartMenu".into(),
        menus::MAP => "MapMenu".into(),
        menus::SLEEP_WAIT => "SleepWaitMenu".into(),
        menus::BARTER => "BarterMenu".into(),
        menus::DIALOGUE => "DialogMenu".into(),
        menus::LOCKPICK => "LockPickMenu".into(),
        menus::REPAIR => "RepairMenu".into(),
        menus::ITEM_MOD => "ItemModMenu".into(),
        menus::LEVEL_UP => "LevelUpMenu".into(),
        menus::TUTORIAL => "TutorialMenu".into(),
        menus::QUANTITY => "QuantityMenu".into(),
        menus::COMPANION_WHEEL => "CompanionWheel".into(),
        nvf_play::script::MENU_CONTAINER => "ContainerMenu".into(),
        _ => nvf_play::script_hud::menu_name(code),
    }
}

fn groups(flags: [bool; 7]) -> Vec<&'static str> {
    flags.into_iter().zip(nvf_play::control::group::NAMES).filter_map(|(locked, name)| locked.then_some(name)).collect()
}

/// [design / file refs] Standard control flags store no caller (`Game::player`);
/// Alt flags retain plugin names (`Game::other.alt_locks`). Do not infer a script.
fn sources(standard: [bool; 7], alt: &std::collections::BTreeMap<String, [bool; 7]>) -> (Vec<Value>, usize) {
    let standard = standard.iter().any(|x| *x).then(|| json!({ "by": "DisablePlayerControls", "script": null, "groups": groups(standard) }));
    let all = standard.into_iter().chain(
        alt.iter()
            .filter(|(_, flags)| flags.iter().any(|x| *x))
            .map(|(plugin, flags)| json!({ "by": "DisablePlayerControlsAlt", "plugin": cut(plugin), "groups": groups(*flags) })),
    );
    let mut kept = Vec::new();
    let mut omitted = 0;
    for source in all {
        if kept.len() < anomaly::CONTROLS_LOCKED_SOURCES {
            kept.push(source);
        } else {
            omitted += 1;
        }
    }
    (kept, omitted)
}

/// [design, #2403] FormID order makes the bounded selection deterministic. Include
/// background quests too: a script holding controls need not belong to a shown quest.
fn quest_stages<'a>(quests: impl Iterator<Item = (&'a fnv_sim::FormId, &'a fnv_game::quest::QuestState)>) -> Vec<Value> {
    quests.filter(|(_, q)| q.running && !q.completed).take(anomaly::CONTROLS_LOCKED_QUESTS).map(|(id, q)| json!({ "quest": format!("{:08X}", id.0), "stage": q.stage() })).collect()
}

/// [design, #2403] Fingerprint logical progress, excluding hover, countdowns and
/// animation clocks. IDs/choice state come from `fnv_game::dialogue::Conversation`.
fn runtime_screen(g: &Game, hash: &mut impl Hasher) {
    g.menu.hash(hash);
    g.ui_prompt.hash(hash);
    g.front_box().map(|b| b.serial).hash(hash);
    dialogue_screen(g.dialogue_serial, g.dialogue.as_ref(), hash);
}

fn dialogue_screen(serial: u32, conversation: Option<&fnv_game::dialogue::Conversation>, hash: &mut impl Hasher) {
    serial.hash(hash);
    if let Some(c) = conversation {
        c.actor.hash(hash);
        c.lines.hash(hash);
        c.line.as_ref().map(|l| (l.topic, l.info, l.response)).hash(hash);
        c.topic_list.hash(hash);
        for choice in &c.choices {
            hash_debug(choice, hash);
        }
    }
}

impl Menus<'_> {
    fn screen(&self, g: Option<&Game>, hash: &mut impl Hasher) -> Option<String> {
        let mut name = None;
        if let Some(model) = self.active.as_ref().and_then(|a| a.0.as_ref()) {
            hash_debug(&(model.mode, model.focus, model.selected, model.scroll, model.category, model.quantity, model.folder), hash);
            for (list, selected) in model.lists.iter().zip(model.selected) {
                // [design / file refs] Counts and selection change on transfers even
                // when the menu's page stays open (`menu::model::Listing`).
                hash_debug(&(list.rows.len(), list.value), hash);
                list.rows.get(selected).map(|r| (r.item, r.count, r.equipped, r.mods)).hash(hash);
            }
            name = Some(menu_name(nvf_play::script::menu_code(model.mode)));
        }
        if let Some(pip) = self.pipboy.as_ref().filter(|p| p.open()) {
            hash_debug(&(pip.page, pip.tabs), hash);
            if let Some(data) = self.data.as_ref().filter(|_| pip.page == nvf_play::pipboy::Page::Data) {
                hash_debug(&data.progress(pip.tabs[pip.page.index()]), hash);
            }
            if let Some(stats) = &self.stats {
                hash_debug(&(stats.status, stats.selected, stats.top, stats.desc, stats.reputation, stats.limb), hash);
            }
            if let Some(repair) = self.repair.as_ref().and_then(|r| r.0.as_ref()) {
                hash_debug(&(repair.first, repair.selected, repair.preview), hash);
                name = Some("RepairMenu".into());
            } else if let Some(item_mod) = self.item_mod.as_ref().and_then(|m| m.0.as_ref()) {
                hash_debug(&(item_mod.first, item_mod.selected), hash);
                name = Some("ItemModMenu".into());
            } else {
                name = Some(format!("Pip-Boy {:?}", pip.page));
            }
        }
        if let Some(wait) = self.wait.as_ref().and_then(|w| w.0.as_ref()) {
            hash_debug(&(wait.sleep, wait.hours, wait.passing.map(|p| (p.asked, p.left))), hash);
            name = Some("SleepWaitMenu".into());
        }
        if let Some(terminal) = self.terminal.as_ref().and_then(|t| t.0.as_ref()) {
            use nvf_play::terminal::Page;
            terminal.reference.hash(hash);
            terminal.base.hash(hash);
            // [design, #2403 / file refs] Fingerprint the terminal.rs page and its
            // visible navigation/result state, excluding hover-sound and end timers.
            match &terminal.page {
                Page::Hacking(h) => {
                    0_u8.hash(hash);
                    hash_debug(&(h.cursor, &h.puzzle.chars, h.puzzle.attempts, &h.puzzle.log, h.puzzle.state), hash);
                    name = Some("HackingMenu".into());
                }
                Page::LockedOut(lines) => {
                    1_u8.hash(hash);
                    lines.hash(hash);
                    name = Some("HackingMenu".into());
                }
                Page::Menu(m) => {
                    2_u8.hash(hash);
                    hash_debug(&(m.terminal, &m.parents, &m.items, &m.labels, m.selected, &m.result, m.note, m.scroll), hash);
                    name = Some("TerminalMenu".into());
                }
            }
        }
        if let Some(progress) = self.levelup.as_ref().and_then(|m| m.progress()) {
            hash_debug(&progress, hash);
            name = Some("LevelUpMenu".into());
        }
        if g.is_some_and(|g| g.dialogue.is_some()) || self.state.as_ref().is_some_and(|s| *s.get() == nvf_play::PlayState::Dialogue) {
            if let Some(dialogue) = &self.dialogue {
                hash_debug(&dialogue.progress(), hash);
            }
        }
        if self.paused.as_ref().is_some_and(|p| p.0) {
            hash_debug(&self.page.as_deref(), hash);
            if let Some(view) = &self.pause_view {
                hash_debug(&view.progress(), hash);
            }
            hash_debug(&self.settings_tab.as_ref().and_then(|t| t.tab.as_ref()), hash);
            name = Some("StartMenu".into());
        }
        if name.is_none() {
            name = g.and_then(|g| g.menu).map(menu_name);
        }
        if name.is_none() {
            name = match self.state.as_ref().map(|s| s.get()) {
                Some(nvf_play::PlayState::Menu) => Some("engine menu".into()),
                Some(nvf_play::PlayState::Dialogue) => Some("DialogMenu".into()),
                _ => None,
            };
        }
        name.hash(hash);
        name
    }
}

pub(super) fn detect(
    real: Res<Time<Real>>,
    loading: Option<Res<nvf_play::loading::Loading>>,
    script: Option<Res<nvf_play::script::ScriptState>>,
    menus: Menus,
    mut presses: Presses,
    mut c: ResMut<Collector>,
) {
    let presses = presses.count();
    let now = real.elapsed_secs_f64();
    let g = script.as_ref().and_then(|s| s.rt.as_ref()).map(|rt| &rt.game);
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let menu = menus.screen(g, &mut hash);
    let dialogue = g.and_then(|g| g.dialogue.as_ref());
    let flags = g.map_or([false; 7], |g| g.controls_locked());
    if let Some(g) = g {
        runtime_screen(g, &mut hash);
    }
    let held = menu.is_some() || dialogue.is_some() || flags.iter().any(|x| *x);
    let progress = (held && !loading.as_ref().is_some_and(|l| l.active())).then_some(LockProgress { screen: hash.finish(), groups: flags });
    let c = &mut *c;
    let Some(held_s) = c.detectors.controls_locked.step(now, progress, presses) else { return };
    let (sources, omitted) = g.map_or_else(|| (Vec::new(), 0), |g| sources(g.player.controls_disabled, &g.other.alt_locks));
    let dialogue = dialogue.map(|d| {
        json!({
            "speaker": format!("{:08X}", d.actor.0),
            "speaker_name": g.map(|g| cut(&g.data.name(d.actor))),
            "topic": d.line.as_ref().map(|l| format!("{:08X}", l.topic.0)),
            "info": d.line.as_ref().map(|l| format!("{:08X}", l.info.0)),
            "response": d.line.as_ref().map(|l| l.response),
            "offered_topics": d.choices.iter().take(anomaly::CONTROLS_LOCKED_TOPICS).map(|c| format!("{:08X}", c.topic.0)).collect::<Vec<_>>(),
        })
    });
    let quests = g.map(|g| quest_stages(g.quests.iter())).unwrap_or_default();
    c.detectors.fire(
        &mut c.summary,
        &c.here,
        c.started.elapsed().as_secs(),
        "controls_locked",
        json!({
            "held_s": held_s.round(), "recent_presses": anomaly::CONTROLS_LOCKED_PRESSES,
            "menu": menu, "dialogue": dialogue, "locked_groups": groups(flags),
            "lock_sources": sources, "lock_sources_omitted": omitted, "quests": quests,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input::keyboard::Key;

    fn detector_app() -> App {
        let now = std::time::Instant::now();
        let mut app = App::new();
        app.init_resource::<Time<Real>>()
            .insert_resource(State::new(nvf_play::PlayState::Menu))
            .add_message::<KeyboardInput>()
            .add_message::<MouseButtonInput>()
            .insert_resource(Collector {
                summary: Default::default(),
                file: Default::default(),
                started: now,
                last_write: now,
                places: Default::default(),
                cells_seen: Default::default(),
                last_space: None,
                space_time: Default::default(),
                start_kind: "test",
                load: None,
                here: Default::default(),
                load_ended: 0.0,
                detectors: Default::default(),
            })
            .add_systems(Update, detect);
        app
    }

    fn sample(app: &mut App, now: f64, presses: usize) {
        let mut time = app.world_mut().resource_mut::<Time<Real>>();
        let delta = now - time.elapsed_secs_f64();
        time.advance_by(std::time::Duration::from_secs_f64(delta));
        for _ in 0..presses {
            app.world_mut().write_message(KeyboardInput {
                key_code: KeyCode::KeyE,
                logical_key: Key::Character("e".into()),
                state: ButtonState::Pressed,
                text: None,
                repeat: false,
                window: Entity::PLACEHOLDER,
            });
        }
        app.update();
    }

    /// [design, #2403] Run the actual collection/detection system, with no reports
    /// plugin, writers, network or game data. Each pair isolates one progress signal.
    fn navigation_resets_detection<R: Resource>(before: R, after: R, ineffective: R, setup: impl FnOnce(&mut World), menu: &str) {
        let mut app = detector_app();
        setup(app.world_mut());
        app.insert_resource(before);
        sample(&mut app, 0.0, 0);
        sample(&mut app, 59.0, 5);
        assert!(app.world().resource::<Collector>().summary.events.is_empty());
        app.insert_resource(after);
        sample(&mut app, 60.0, 5);
        assert!(app.world().resource::<Collector>().summary.events.is_empty(), "navigation at the old deadline resets the watch");
        sample(&mut app, 119.0, 5);
        assert!(app.world().resource::<Collector>().summary.events.is_empty(), "59 seconds since progress");
        app.insert_resource(ineffective);
        sample(&mut app, 120.0, 5);
        let events = &app.world().resource::<Collector>().summary.events;
        assert_eq!(events.len(), 1, "ineffective input must not reset the timer");
        assert_eq!(events[0]["kind"], "controls_locked");
        assert_eq!(events[0]["menu"], menu);
        assert_eq!(events[0]["held_s"], 60.0);
        sample(&mut app, 180.0, 5);
        assert_eq!(app.world().resource::<Collector>().summary.events.len(), 1, "once per unchanged lock");
    }

    fn terminal_fixture(step: usize) -> nvf_play::terminal::TerminalScreen {
        use fnv_sim::FormId;
        use nvf_play::terminal::{MenuView, Page, Session, TerminalScreen};
        let mut m = MenuView {
            terminal: FormId(42),
            parents: Vec::new(),
            items: vec![0, 1],
            labels: vec!["Synthetic first row".into(), "Synthetic second row".into()],
            selected: 0,
            hidden: false,
            result: String::new(),
            note: None,
            scroll: 0,
            server: 0,
            welcome: String::new(),
        };
        if step >= 1 {
            m.selected = 1;
        }
        if step >= 2 {
            m.parents.push((m.terminal, m.selected));
            m.terminal = FormId(43);
        }
        if step >= 3 {
            m.note = Some(FormId(44));
        }
        if step >= 4 {
            m.scroll = 1;
        }
        if step >= 5 {
            m.result = "Synthetic result".into();
        }
        let page = if step == 6 { Page::LockedOut(["Synthetic lockout".into(), String::new()]) } else { Page::Menu(m) };
        TerminalScreen(Some(Session { reference: FormId(41), base: FormId(42), page }))
    }

    #[test]
    fn detect_terminal_navigation_resets_but_ineffective_presses_fire() {
        for step in 1..=6 {
            navigation_resets_detection(terminal_fixture(step - 1), terminal_fixture(step), terminal_fixture(step), |_| {}, if step == 6 { "HackingMenu" } else { "TerminalMenu" });
        }
    }

    #[test]
    fn detect_hacking_selection_and_results_reset_but_timers_do_not() {
        use nvf_play::terminal::report_progress_fixture as fixture;
        for step in 1..=2 {
            navigation_resets_detection(fixture(step - 1), fixture(step), fixture(if step == 2 { 3 } else { step }), |_| {}, "HackingMenu");
        }
    }

    #[test]
    fn detect_builtin_screens_ignore_idle_time_until_presses_arrive() {
        for screen in 0..4 {
            let mut app = detector_app();
            match screen {
                0 => {
                    app.insert_resource(terminal_fixture(0));
                }
                1 => {
                    app.insert_resource(nvf_play::levelup::report_progress_fixture(0));
                }
                2 => {
                    app.insert_resource(nvf_play::dialogue::report_progress_fixture(0));
                    app.insert_resource(State::new(nvf_play::PlayState::Dialogue));
                }
                _ => {
                    let mut pip = nvf_play::pipboy::PipBoy::default();
                    pip.phase = nvf_play::pipboy::Phase::Up;
                    pip.page = nvf_play::pipboy::Page::Data;
                    app.insert_resource(pip);
                    app.insert_resource(nvf_play::pipboy::data::report_progress_fixture(0));
                }
            }
            sample(&mut app, 0.0, 0);
            sample(&mut app, 120.0, 0);
            assert!(app.world().resource::<Collector>().summary.events.is_empty());
            sample(&mut app, 121.0, 5);
            assert_eq!(app.world().resource::<Collector>().summary.events.len(), 1);
        }
    }

    #[test]
    fn detect_levelup_navigation_and_allocations_reset_but_hover_does_not() {
        use nvf_play::levelup::report_progress_fixture as fixture;
        for step in 1..=5 {
            navigation_resets_detection(fixture(step - 1), fixture(step), fixture(if step == 5 { 6 } else { step }), |_| {}, "LevelUpMenu");
        }
    }

    #[test]
    fn detect_dialogue_selection_resets_but_boundary_presses_and_timers_do_not() {
        use nvf_play::dialogue::report_progress_fixture as fixture;
        navigation_resets_detection(
            fixture(0),
            fixture(1),
            fixture(2),
            |w| {
                w.insert_resource(State::new(nvf_play::PlayState::Dialogue));
            },
            "DialogMenu",
        );
    }

    #[test]
    fn detect_data_note_scroll_and_list_navigation_reset_but_counters_do_not() {
        use nvf_play::pipboy::{data::report_progress_fixture as fixture, Page, Phase, PipBoy};
        for step in 1..=3 {
            navigation_resets_detection(
                fixture(step - 1),
                fixture(step),
                fixture(if step == 3 { 4 } else { step }),
                |w| {
                    let mut pip = PipBoy::default();
                    pip.phase = Phase::Up;
                    pip.page = Page::Data;
                    pip.tabs[Page::Data.index()] = 3;
                    w.insert_resource(pip);
                },
                "Pip-Boy Data",
            );
        }
    }

    #[derive(Resource, Default)]
    struct Sample {
        screen: u64,
        presses: usize,
    }

    #[test]
    fn engine_menu_pages_reset_the_watch() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).insert_resource(nvf_play::pause::Paused(true)).init_resource::<nvf_play::pause::Page>().init_resource::<Sample>().add_systems(
            Update,
            |menus: Menus, mut sample: ResMut<Sample>| {
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                assert_eq!(menus.screen(None, &mut hash).as_deref(), Some("StartMenu"));
                sample.screen = hash.finish();
            },
        );
        app.update();
        let first = app.world().resource::<Sample>().screen;
        let mut watch = anomaly::ControlsLocked::default();
        watch.step(0.0, Some(LockProgress { screen: first, groups: [false; 7] }), anomaly::CONTROLS_LOCKED_PRESSES);
        *app.world_mut().resource_mut::<nvf_play::pause::Page>() = nvf_play::pause::Page::Settings { selected: None };
        app.update();
        let second = app.world().resource::<Sample>().screen;
        assert_ne!(first, second);
        assert_eq!(watch.step(59.0, Some(LockProgress { screen: second, groups: [false; 7] }), 0), None);
        assert_eq!(watch.step(60.0, Some(LockProgress { screen: second, groups: [false; 7] }), anomaly::CONTROLS_LOCKED_PRESSES), None);
    }

    #[test]
    fn input_collection_counts_physical_edges_without_synthesized_menu_keys() {
        use bevy::input::gamepad::{RawGamepadButtonChangedEvent, RawGamepadEvent};
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::input::InputPlugin)).init_resource::<Sample>().add_systems(
            Update,
            (|mut keys: ResMut<ButtonInput<KeyCode>>| keys.press(KeyCode::Enter), |mut presses: Presses, mut sample: ResMut<Sample>| sample.presses = presses.count()).chain(),
        );
        let pad = app.world_mut().spawn(Gamepad::default()).id();
        app.world_mut().write_message(RawGamepadEvent::Button(RawGamepadButtonChangedEvent::new(pad, GamepadButton::South, 1.0)));
        app.world_mut().write_message(MouseButtonInput { button: MouseButton::Left, state: ButtonState::Pressed, window: Entity::PLACEHOLDER });
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::KeyE,
            logical_key: Key::Character("e".into()),
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
        app.update();
        assert!(app.world().resource::<ButtonInput<KeyCode>>().just_pressed(KeyCode::Enter));
        assert_eq!(app.world().resource::<Sample>().presses, 3);
        app.update();
        assert_eq!(app.world().resource::<Sample>().presses, 0, "held buttons have no new edges");
    }

    #[test]
    fn keyboard_repeats_and_releases_are_not_presses() {
        let mut event =
            KeyboardInput { key_code: KeyCode::KeyE, logical_key: Key::Character("e".into()), state: ButtonState::Pressed, text: None, repeat: false, window: Entity::PLACEHOLDER };
        assert!(key_press(&&event));
        event.repeat = true;
        assert!(!key_press(&&event));
        event.repeat = false;
        event.state = ButtonState::Released;
        assert!(!key_press(&&event));
    }

    #[test]
    fn running_quest_stages_are_bounded() {
        let mut quests = std::collections::BTreeMap::new();
        for i in 0..20 {
            let mut quest = fnv_game::quest::QuestState { running: i != 0, completed: i == 1, ..Default::default() };
            quest.stages_done.insert(i as i32 * 10);
            quests.insert(fnv_sim::FormId(i), quest);
        }
        let stages = quest_stages(quests.iter());
        assert_eq!(stages.len(), anomaly::CONTROLS_LOCKED_QUESTS);
        assert_eq!(stages[0], json!({ "quest": "00000002", "stage": 20 }));
        assert_eq!(stages.last().unwrap()["quest"], "0000000B");
    }

    #[test]
    fn labels_and_group_names_are_bounded() {
        assert_eq!(cut(&"é".repeat(200)).chars().count(), anomaly::CONTROLS_LOCKED_LABEL_CHARS);
        assert_eq!(groups([true, false, false, false, true, false, false]), ["movement", "looking"]);
        assert_eq!(menu_name(fnv_game::game::menus::MESSAGE), "MessageMenu");
        assert_eq!(menu_name(65535), "Menu 65535");
    }

    #[test]
    fn lock_sources_keep_known_provenance_and_cap_plugin_labels() {
        let mut alt = std::collections::BTreeMap::new();
        alt.insert("inactive".into(), [false; 7]);
        for i in 0..20 {
            alt.insert(format!("{i:02}{}", "x".repeat(200)), [true; 7]);
        }
        let (sources, omitted) = sources([true, false, false, false, false, false, false], &alt);
        assert_eq!(sources.len(), anomaly::CONTROLS_LOCKED_SOURCES);
        assert_eq!(omitted, 11);
        assert_eq!(sources[0]["by"], "DisablePlayerControls");
        assert_eq!(sources[0]["script"], Value::Null);
        assert_eq!(sources[0]["groups"], json!(["movement"]));
        assert_eq!(sources[1]["by"], "DisablePlayerControlsAlt");
        assert_eq!(sources[1]["plugin"].as_str().unwrap().len(), anomaly::CONTROLS_LOCKED_LABEL_CHARS);
    }

    #[test]
    fn new_dialogue_lines_and_choices_are_progress_but_countdowns_are_not() {
        use fnv_game::dialogue::{Choice, Conversation, Line};
        use fnv_sim::FormId;
        let fingerprint = |c: &Conversation| {
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            dialogue_screen(1, Some(c), &mut hash);
            hash.finish()
        };
        let mut c = Conversation {
            actor: FormId(42),
            through: None,
            line: Some(Line { topic: FormId(43), info: FormId(44), response: 0, remaining: 10.0 }),
            choices: Vec::new(),
            topic_list: false,
            lines: 1,
            then: None,
            ending: false,
        };
        let first = fingerprint(&c);
        c.line.as_mut().unwrap().remaining = 9.0;
        assert_eq!(first, fingerprint(&c));
        c.line.as_mut().unwrap().response = 1;
        assert_ne!(first, fingerprint(&c));
        c.line = None;
        let choosing = fingerprint(&c);
        c.choices.push(Choice { topic: FormId(45), info: FormId(46), chars: 4, prompt: true, goodbye: false, darkened: false, challenge: None, dumb: false });
        assert_ne!(choosing, fingerprint(&c));
        c.topic_list = true;
        assert_ne!(choosing, fingerprint(&c));
        c.lines += 1;
        assert_ne!(choosing, fingerprint(&c));
    }
}
