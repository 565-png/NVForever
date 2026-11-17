//! Process-local original-profile startup and real damage-rule controls.
use super::*;
use crate::{
    settings::SettingsFile,
    tweaks::{
        stewie_profile::{self, ProfileState},
        TweakSettings,
    },
};
use fnv_script::vm::XVal;
use std::path::PathBuf;

struct OwnedInputs {
    root: PathBuf,
    data: PathBuf,
    nvf: PathBuf,
}
impl OwnedInputs {
    fn new() -> Self {
        let base = std::env::var_os("NVF_STEWIE_PROFILE_HOME")
            .expect("set NVF_STEWIE_PROFILE_HOME to a dedicated test directory");
        let base = PathBuf::from(base);
        assert!(
            base.is_absolute(),
            "process-owned fixture root must be absolute"
        );
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = base.join(format!("owned-{}-{stamp}", std::process::id()));
        let data = root.join("Data");
        std::fs::create_dir_all(data.join("NVSE/Plugins")).unwrap();
        std::fs::create_dir(data.join("NVSE/Plugins/scripts")).unwrap();
        std::fs::write(data.join("NVSE/Plugins/scripts/gr_owned_profile.txt"), "SetModLocalData \"profile.restart.rate\" (StewVal \"bCritChanceIgnoresFireRate:Tweaks\")\n").unwrap();
        std::fs::write(data.join("NVSE/Plugins/scripts/ln_owned_profile.txt"), "SetModLocalData \"profile.load.rate\" (StewVal \"bCritChanceIgnoresFireRate:Tweaks\")\n").unwrap();
        let nvf = root.join("NVForever.ini");
        fnv_game::mod_files::set_roots(fnv_game::mod_files::Roots {
            read: vec![data.clone()],
            write: None,
            ..default()
        });
        Self { root, data, nvf }
    }
    fn profile(&self, ignore_rate: u8, scale: f64) {
        std::fs::write(self.data.join("NVSE/Plugins/nvse_stewie_tweaks.ini"), format!("[Tweaks]\nbCritChanceIgnoresFireRate={ignore_rate}\nbScaleCriticalDamage=1\n[Critical Hits]\nfCriticalDamageMult={scale}\n")).unwrap();
    }
}
impl Drop for OwnedInputs {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.data.join("NVSE/Plugins/nvse_stewie_tweaks.ini"));
        let _ = std::fs::remove_file(&self.nvf);
        for name in ["gr_owned_profile.txt", "ln_owned_profile.txt"] {
            let _ = std::fs::remove_file(self.data.join("NVSE/Plugins/scripts").join(name));
        }
        let _ = std::fs::remove_dir(self.data.join("NVSE/Plugins/scripts"));
        for path in [
            self.data.join("NVSE/Plugins"),
            self.data.join("NVSE"),
            self.data.clone(),
            self.root.clone(),
        ] {
            let _ = std::fs::remove_dir(path);
        }
    }
}
fn fresh(
    settings: &mut TweakSettings,
    state: &mut ProfileState,
    file: &SettingsFile,
) -> (Runtime, SimState) {
    let original = ScriptState::over_no_plugins().rt.unwrap();
    let data = original.game.data;
    let forms = original.game.world.forms;
    let refs = original.game.world.placed;
    let mut sim = SimState {
        forms,
        refs,
        world: World::new(forms, refs, fnv_sim::SimConfig::default()),
    };
    let rt = Runtime::new(
        data,
        &mut sim,
        true,
        false,
        Some((settings, state, Some(file))),
    );
    (rt, sim)
}
fn source(rt: &mut Runtime, body: &str) -> XVal {
    rt.game
        .run_query(
            &format!("Begin Function {{}}\n{body}\nEnd\n"),
            Default::default(),
        )
        .unwrap()
}
fn damage(tweaks: &fnv_game::tweaks::Tweaks) -> fnv_combat::damage::Damage {
    use fnv_combat::{
        damage::{self, Attacker, HitInput, HitPart, NoPerks, Target},
        data::WeaponStats,
        tuning::{CombatTuning, Rules},
    };
    let mut rules = Rules::defaults();
    crate::combat::apply_critical_tweaks(&mut rules, tweaks);
    let weapon = WeaponStats {
        automatic: true,
        fire_rate: 10.0,
        crit_mult: 1.0,
        crit_damage: 20,
        base_damage: 10,
        ..default()
    };
    damage::hit(
        &rules,
        &CombatTuning::default(),
        &HitInput {
            weapon: &weapon,
            projectiles: 1,
            condition: 1.0,
            ammo_effects: &[],
            part: HitPart::default(),
            power: 1.0,
        },
        &Attacker::default(),
        &Target::default(),
        &NoPerks,
        &NoPerks,
        0.0,
    )
}

#[test]
#[ignore = "process-local ModFiles roots: run this exact selector alone with dedicated NVF_STEWIE_PROFILE_HOME and --test-threads=1"]
fn original_stat_profile_reaches_startup_source_damage_settings_and_restore_without_version_uplift()
{
    let files = OwnedInputs::new();
    let file = SettingsFile(Some(files.nvf.clone()));
    let mut settings = TweakSettings::default();
    let mut state = ProfileState::default();
    let (no_profile, _) = fresh(&mut settings, &mut state, &file);
    assert_eq!(no_profile.game.tweaks, fnv_game::tweaks::Tweaks::default());
    let ordinary = damage(&no_profile.game.tweaks);
    assert!(ordinary.critical && ordinary.crit_bonus > 0.0);
    files.profile(1, 1.5);
    let mut settings = TweakSettings::default();
    let mut state = ProfileState::default();
    let (mut rt, mut sim) = fresh(&mut settings, &mut state, &file);
    assert_eq!(
        source(
            &mut rt,
            "SetFunctionValue (StewVal \"bCritChanceIgnoresFireRate:Tweaks\")"
        ),
        XVal::Num(1.0)
    );
    assert_eq!(
        source(
            &mut rt,
            "SetFunctionValue (StewVal \"fCriticalDamageMult:Critical Hits\")"
        ),
        XVal::Num(1.5)
    );
    script_runner(&mut rt, &mut sim, Moment::Restart);
    script_runner(&mut rt, &mut sim, Moment::NewGame);
    assert_eq!(
        source(
            &mut rt,
            "SetFunctionValue (GetModLocalData \"profile.restart.rate\")"
        ),
        XVal::Num(1.0)
    );
    assert_eq!(
        source(
            &mut rt,
            "SetFunctionValue (GetModLocalData \"profile.load.rate\")"
        ),
        XVal::Num(1.0)
    );
    let imported = damage(&rt.game.tweaks);
    assert_eq!(imported.crit_chance, ordinary.crit_chance * 10.0);
    assert_eq!(imported.crit_bonus, ordinary.crit_bonus * 1.5);
    assert!(imported.dealt > ordinary.dealt);
    // Preserve the original dependency predicate and a real message branch. This
    // bounded behavior repair is not a version-based popup bypass.
    assert_eq!(source(&mut rt, "if eval (GetPluginVersion \"lStewieAl's Tweaks\") < 960\nMessageBoxEx \"Unsupported compatibility level\"\nSetFunctionValue 0\nreturn\nendif\nSetFunctionValue 1"), XVal::Num(0.0));
    assert!(
        rt.game.front_box().is_some(),
        "the real guard warning stays queued; it is never suppressed or dismissed"
    );
    assert_eq!(
        source(
            &mut rt,
            "SetFunctionValue (GetPluginVersion \"lStewieAl's Tweaks\")"
        ),
        XVal::Num(941.0)
    );
    let saved = with_world(&mut rt, &mut sim, |rt| rt.game.snapshot());
    let bytes = serde_json::to_vec(&saved).unwrap();
    assert!(!serde_json::from_slice::<serde_json::Value>(&bytes)
        .unwrap()
        .as_object()
        .unwrap()
        .contains_key("tweaks"));
    settings.game.crit_damage_scale = 2.0;
    rt.restore_save(&mut sim, serde_json::from_slice(&bytes).unwrap())
        .unwrap();
    stewie_profile::handoff(&mut rt.game, &settings);
    script_runner(&mut rt, &mut sim, Moment::Restart);
    script_runner(&mut rt, &mut sim, Moment::Load);
    assert_eq!(
        source(
            &mut rt,
            "SetFunctionValue (GetModLocalData \"profile.restart.rate\")"
        ),
        XVal::Num(1.0)
    );
    assert_eq!(
        source(
            &mut rt,
            "SetFunctionValue (GetModLocalData \"profile.load.rate\")"
        ),
        XVal::Num(1.0)
    );
    assert_eq!(
        damage(&rt.game.tweaks).crit_bonus,
        ordinary.crit_bonus * 2.0,
        "saved game defaults do not replace current application settings before Restart"
    );
    files.profile(0, 0.5);
    stewie_profile::prepare(&mut rt.game, &mut settings, &mut state, Some(&file));
    assert!(
        rt.game.tweaks.crit_ignores_fire_rate,
        "runtime INI changes are not imported over live settings in this startup-only subset"
    );
    assert_eq!(rt.game.tweaks.crit_damage_scale, 2.0);
    assert_eq!(
        source(
            &mut rt,
            "SetFunctionValue (StewVal \"bCritChanceIgnoresFireRate:Tweaks\")"
        ),
        XVal::Num(1.0),
        "recognized loaded setting follows actual rules after a runtime original-file edit"
    );
    assert_eq!(
        source(
            &mut rt,
            "SetFunctionValue (StewieGetValue \"fCriticalDamageMult:Critical Hits\")"
        ),
        XVal::Num(2.0)
    );
    assert_eq!(
        source(
            &mut rt,
            "SetFunctionValue (StewVal \"bScaleCriticalDamage:Tweaks\")"
        ),
        XVal::Num(1.0),
        "the unrepresented enable switch retains its original-file value"
    );
    // A real stored NVForever choice, including explicit off/default-valued keys,
    // takes precedence. Do not infer whether legacy keys were user-selected.
    std::fs::write(
        &files.nvf,
        "[Settings]\ntweak-crit-ignores-fire-rate=0\ntweak-crit-damage-scale=0.75\n",
    )
    .unwrap();
    let mut app = App::new();
    app.add_plugins(crate::tweaks::TweaksPlugin);
    app.insert_resource(file.clone());
    crate::settings::apply_stored(app.world_mut());
    let mut chosen = app.world().resource::<TweakSettings>().clone();
    assert_eq!(
        chosen.game.crit_damage_scale, 1.5,
        "the existing settings reader really applies slider part0.75 as x1.5"
    );
    let (chosen_rt, _) = fresh(&mut chosen, &mut ProfileState::default(), &file);
    assert!(!chosen_rt.game.tweaks.crit_ignores_fire_rate);
    assert_eq!(chosen_rt.game.tweaks.crit_damage_scale, 1.5);
}

mod current_profile;
mod inventory_settings;
