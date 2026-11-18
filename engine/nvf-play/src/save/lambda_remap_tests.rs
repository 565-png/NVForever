//! Actual container reader controls for saves carrying runtime lambda handles.
use super::*;
use fnv_game::game::InstKey;
use fnv_script::vm::{Value, XVal};
use fnv_sim::FormId;

const PLACED_BASE: u32 = 0xD10;

struct OwnedDir(PathBuf);
impl Drop for OwnedDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A real snapshot over one plugin: a script variable holding a plugin record and,
/// with `lambda`, a second one holding a lambda the runtime returned.
fn saved(lambda: bool) -> (Vec<PluginEntry>, SaveBody, Option<u32>) {
    use crate::script_draw::tests::{play_over, record, sub, top};
    let play = play_over(&[top(b"STAT", &record(b"STAT", 0, PLACED_BASE, &[sub(b"EDID", b"OwnedRemapBase\0".to_vec())]))]);
    let mut state = ScriptState::over_plugins(play.lo.plugins);
    let g = &mut state.rt.as_mut().unwrap().game;
    let owner = InstKey::Ref { reference: FormId::PLAYER_REF };
    g.store(owner, None, 1, Value::Ref(PLACED_BASE), "owned remap fixture", false);
    let token = lambda.then(|| {
        let made = g.run_query("Begin Function {}\nSetFunctionValue (Begin Function {}\nSetFunctionValue 17\nEnd)\nEnd", Default::default()).unwrap();
        let XVal::Ref(token) = made else { panic!("the runtime returns a lambda: {made:?}") };
        g.store(owner, None, 2, Value::Ref(token), "owned remap fixture", false);
        token
    });
    let body = SaveBody { format: BODY_FORMAT.into(), game: g.checked_snapshot().unwrap(), engine: EngineSnapshot { format: ENGINE_FORMAT.into(), ..default() } };
    assert_eq!(body.game.lambdas.iter().map(|l| l.handle).collect::<Vec<_>>(), token.map(Some).into_iter().collect::<Vec<_>>());
    (vec![PluginEntry { name: "Test.esm".into(), size: 0 }], body, token)
}

fn write(label: &str, plugins: &[PluginEntry], body: &SaveBody) -> (OwnedDir, PathBuf) {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("nvf-lambda-remap-{label}-{}-{sequence}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("lambda.nvfsave");
    let mut header = SaveHeader::new(SaveKind::Manual);
    header.plugins = plugins.to_vec();
    fnv_save::write(&path, &header, &[], &serde_json::to_vec(body).unwrap(), false).unwrap();
    (OwnedDir(dir), path)
}

/// Every saved script-variable reference token (`{"ref": n}`) in the game snapshot.
fn ref_tokens(body: &SaveBody) -> Vec<u32> {
    fn walk(v: &serde_json::Value, out: &mut Vec<u32>) {
        match v {
            serde_json::Value::Object(o) if o.len() == 1 && o.get("ref").is_some_and(|n| n.is_u64()) => out.push(o["ref"].as_u64().unwrap() as u32),
            serde_json::Value::Object(o) => o.values().for_each(|e| walk(e, out)),
            serde_json::Value::Array(a) => a.iter().for_each(|e| walk(e, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(&serde_json::to_value(&body.game).unwrap(), &mut out);
    out.sort_unstable();
    out
}

fn other() -> PluginEntry {
    PluginEntry { name: "OwnedOther.esm".into(), size: 0 }
}

#[test]
fn a_lambda_save_keeps_its_handle_with_identical_or_appended_plugins() {
    let (plugins, body, token) = saved(true);
    let token = token.unwrap();
    assert!(token >= fnv_game::game::LAMBDA_BASE);
    let (_dir, path) = write("kept", &plugins, &body);
    let mut appended = plugins.clone();
    appended.push(other());
    for current in [plugins, appended] {
        let (_, read, order) = read_for_load(&path, &current).unwrap();
        assert!(order.is_none());
        assert_eq!(read.game.lambdas.iter().map(|l| l.handle).collect::<Vec<_>>(), vec![Some(token)]);
        assert_eq!(ref_tokens(&read), vec![PLACED_BASE, token]);
    }
}

#[test]
fn a_lambda_save_is_refused_with_a_reordered_or_missing_plugin_list() {
    let (plugins, body, _) = saved(true);
    let mut legacy = body.clone();
    legacy.game.lambdas[0].handle = None;
    for (label, body) in [("explicit", body), ("legacy", legacy)] {
        let (_dir, path) = write(label, &plugins, &body);
        let bytes = std::fs::read(&path).unwrap();
        let mut moved = vec![other()];
        moved.extend(plugins.clone());
        for current in [moved, Vec::new()] {
            let Err(error) = read_for_load(&path, &current) else { panic!("{label}: a reordered lambda save must be refused") };
            assert!(error.contains("runtime lambda definitions") && error.contains("lambda.nvfsave"), "{label}: {error}");
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
}

#[test]
fn a_lambda_free_save_still_remaps_with_a_reordered_plugin_list() {
    let (plugins, body, token) = saved(false);
    assert!(token.is_none());
    let (_dir, path) = write("plain", &plugins, &body);
    let (_, read, order) = read_for_load(&path, &plugins).unwrap();
    assert!(order.is_none());
    assert_eq!(ref_tokens(&read), vec![PLACED_BASE]);
    let mut moved = vec![other()];
    moved.extend(plugins);
    let (_, read, order) = read_for_load(&path, &moved).unwrap();
    assert!(order.is_some());
    assert!(read.game.lambdas.is_empty());
    assert_eq!(ref_tokens(&read), vec![0x0100_0000 | PLACED_BASE]);
}
