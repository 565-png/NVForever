//! Authored page input consumers; docs/spec-notes/mcm-page-key-events.md.
use super::*;
use bevy::input::mouse::MouseScrollUnit;
use crate::script::ScriptState;
use crate::tiles::mods::{self, ModsPage};
use fnv_game::game::{menus, RunCtx};
use fnv_script::vm::XVal;

fn page(mut state: ResMut<ScriptState>, mut page: ResMut<ModsPage>) {
    mods::tick(&mut state.rt.as_mut().unwrap().game, &mut page);
}

fn app() -> (App, Entity) {
    let mut app = App::new();
    app.init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ScriptBlocked>()
        .init_resource::<ModsPage>()
        .add_message::<MouseWheel>()
        .insert_resource(ScriptState::over_no_plugins())
        .add_systems(Update, (script_input, page).chain());
    let window = app.world_mut().spawn(PrimaryWindow).id();
    let mut state = app.world_mut().resource_mut::<ScriptState>();
    let game = &mut state.rt.as_mut().unwrap().game;
    let files = fnv_ui::MemSource::default().with("owned.xml", r#"
      <menu name="StartMenu"><rect name="MCM"><_Menu>1</_Menu>
        <rect name="MCM_ModList"><_offset>1</_offset><_visible>3</_visible>
          <_count>6</_count><_searchKeys>0</_searchKeys><_released>0</_released><_allDown>0</_allDown>
        </rect></rect></menu>"#);
    game.ui_tree.load_menu(&files, "owned.xml").unwrap();
    // Independent consumers of the same documented key-event and MCM tile APIs;
    // none of the installed mod's scripts or XML are test fixtures.
    let handler = r#"Begin Function {int key}
      if MenuMode 1013 == 0
        return
      endif
      if key == 264
        if GetMCMFloat 5 0 "_offset" > 0
          SetMCMFloat 5 0 "_offset" ((GetMCMFloat 5 0 "_offset") - 1)
        endif
      elseif key == 265
        if (GetMCMFloat 5 0 "_offset") + (GetMCMFloat 5 0 "_visible") < GetMCMFloat 5 0 "_count"
          SetMCMFloat 5 0 "_offset" ((GetMCMFloat 5 0 "_offset") + 1)
        endif
      else
        SetMCMFloat 5 0 "_searchKeys" ((GetMCMFloat 5 0 "_searchKeys") + 1)
      endif
    End"#;
    for code in [264, 265, 33] {
        let source = format!("Begin Function {{}}\nSetFunctionValue (SetOnKeyDownEventHandler ({handler}) 1 {code})\nEnd");
        assert_eq!(game.run_query(&source, RunCtx::default()).unwrap(), XVal::Num(1.0));
    }
    let source = r#"Begin Function {}
      SetFunctionValue (SetOnKeyUpEventHandler (Begin Function {int key}
        if MenuMode 1013
          SetMCMFloat 5 0 "_released" key
        endif
      End) 1 33)
    End"#;
    assert_eq!(game.run_query(source, RunCtx::default()).unwrap(), XVal::Num(1.0));
    let source = r#"Begin Function {}
      SetFunctionValue (SetOnKeyDownEventHandler (Begin Function {int key}
        SetMCMFloat 5 0 "_allDown" ((GetMCMFloat 5 0 "_allDown") + 1)
      End) 1 33)
    End"#;
    assert_eq!(game.run_query(source, RunCtx::default()).unwrap(), XVal::Num(1.0));
    drop(state);
    (app, window)
}

fn wheel(app: &mut App, window: Entity, y: f32) {
    app.world_mut().write_message(MouseWheel {
        unit: MouseScrollUnit::Line, x: 0.0, y, window,
        phase: bevy::input::touch::TouchPhase::Moved,
    });
}

fn value(app: &App, field: &str) -> f64 {
    app.world().resource::<ScriptState>().rt.as_ref().unwrap().game.ui_tree
        .get_float(&format!("StartMenu/MCM/MCM_ModList/{field}")).unwrap()
}

#[test]
fn mcm_page_wheel_reaches_source_registered_list_handler_and_respects_edges() {
    let (mut app, window) = app();
    wheel(&mut app, window, 1.0);
    app.update();
    assert_eq!(value(&app, "_offset"), 0.0);
    // A fresh down direction is an edge even without an empty frame between.
    wheel(&mut app, window, -1.0);
    app.update();
    assert_eq!(value(&app, "_offset"), 1.0);
    wheel(&mut app, window, -1.0);
    app.update();
    assert_eq!(value(&app, "_offset"), 1.0, "held direction is not another down edge");
    for expected in [2.0, 3.0, 3.0] {
        app.update();
        wheel(&mut app, window, -1.0);
        app.update();
        assert_eq!(value(&app, "_offset"), expected, "list bounds stay consumer-owned");
    }
    let game = &app.world().resource::<ScriptState>().rt.as_ref().unwrap().game;
    assert_eq!(game.time, 0.0, "page input does not run the world");
    assert!(!game.menu_open(menus::START), "temporary page context is restored");
}

#[test]
fn mcm_page_search_gets_disabled_key_down_up_and_no_replay_after_reentry() {
    let (mut app, _) = app();
    app.world_mut().resource_mut::<ScriptState>().rt.as_mut().unwrap().game.input.disabled_keys.insert(33);
    // A box already over the held world stays there through the page's temporary
    // start-menu context and input dispatch.
    app.world_mut().resource_mut::<ScriptState>().rt.as_mut().unwrap().game.menu = Some(menus::MESSAGE);
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyF);
    app.update();
    assert_eq!(value(&app, "_searchKeys"), 1.0);
    app.update();
    assert_eq!(value(&app, "_searchKeys"), 1.0);
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().release(KeyCode::KeyF);
    app.update();
    assert_eq!(value(&app, "_released"), 33.0);
    app.world_mut().insert_resource(ModsPage::default());
    app.update();
    assert_eq!(value(&app, "_searchKeys"), 1.0);
    app.world_mut().resource_mut::<ButtonInput<KeyCode>>().press(KeyCode::KeyF);
    app.update();
    assert_eq!(value(&app, "_searchKeys"), 2.0);
    assert_eq!(value(&app, "_allDown"), 2.0);
    // The regular frame uses the same edge tracker, so leaving the page while held
    // cannot redeliver the already-consumed key to a handler in the next frame.
    let mut state = app.world_mut().resource_mut::<ScriptState>();
    let game = &mut state.rt.as_mut().unwrap().game;
    game.tick();
    assert_eq!(game.ui_tree.get_float("StartMenu/MCM/MCM_ModList/_searchKeys"), Some(2.0));
    assert_eq!(game.ui_tree.get_float("StartMenu/MCM/MCM_ModList/_allDown"), Some(2.0));
    assert_eq!(game.menu, Some(menus::MESSAGE));
}
