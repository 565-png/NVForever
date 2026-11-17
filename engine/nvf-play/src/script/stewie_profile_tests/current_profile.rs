//! Same existing APIs on both sides: loaded critical settings and real damage.
use super::*;

#[test]
#[ignore = "process-local roots: run exact selector alone with dedicated NVF_STEWIE_PROFILE_HOME and --test-threads=1"]
fn current_explicit_profile_source_values_match_actual_critical_rules_without_rewriting_ini() {
    let files = OwnedInputs::new();
    files.profile(1, 1.75);
    std::fs::write(
        &files.nvf,
        "[Settings]\ntweak-crit-ignores-fire-rate=0\ntweak-crit-damage-scale=0.50\n",
    )
    .unwrap();
    let original = std::fs::read(files.data.join("NVSE/Plugins/nvse_stewie_tweaks.ini")).unwrap();
    let preferences = std::fs::read(&files.nvf).unwrap();
    let file = SettingsFile(Some(files.nvf.clone()));
    let mut app = App::new();
    app.add_plugins(crate::tweaks::TweaksPlugin);
    app.insert_resource(file.clone());
    crate::settings::apply_stored(app.world_mut());
    let mut settings = app.world().resource::<TweakSettings>().clone();
    assert!(!settings.game.crit_ignores_fire_rate);
    assert_eq!(settings.game.crit_damage_scale, 1.0);
    let (mut rt, mut sim) = fresh(&mut settings, &mut ProfileState::default(), &file);
    let parsed_original = rt.game.stewie_stat_profile().unwrap();
    assert_eq!(parsed_original.crit_ignores_fire_rate, Some(true));
    assert_eq!(
        parsed_original.crit_damage_scale,
        Some(1.75),
        "the original file and existing preference really disagree"
    );
    let real_hit = damage(&rt.game.tweaks);
    assert_eq!(
        real_hit.crit_bonus,
        damage(&fnv_game::tweaks::Tweaks::default()).crit_bonus
    );
    assert_eq!(source(&mut rt, "SetFunctionValue (StewVal \"bCritChanceIgnoresFireRate:Tweaks\")"), XVal::Num(0.0), "recognized loaded boolean must agree with the actual critical rule, not a conflicting file default");
    assert_eq!(
        source(
            &mut rt,
            "SetFunctionValue (StewieGetValue \"fCriticalDamageMult:Critical Hits\")"
        ),
        XVal::Num(1.0),
        "recognized loaded scalar must agree with actual critical damage"
    );
    script_runner(&mut rt, &mut sim, Moment::Restart);
    script_runner(&mut rt, &mut sim, Moment::NewGame);
    assert_eq!(
        source(
            &mut rt,
            "SetFunctionValue (GetModLocalData \"profile.restart.rate\")"
        ),
        XVal::Num(0.0)
    );
    assert_eq!(
        source(
            &mut rt,
            "SetFunctionValue (GetModLocalData \"profile.load.rate\")"
        ),
        XVal::Num(0.0)
    );
    assert_eq!(
        std::fs::read(files.data.join("NVSE/Plugins/nvse_stewie_tweaks.ini")).unwrap(),
        original
    );
    assert_eq!(std::fs::read(&files.nvf).unwrap(), preferences);
    assert_eq!(
        source(
            &mut rt,
            "SetFunctionValue (GetPluginVersion \"lStewieAl's Tweaks\")"
        ),
        XVal::Num(941.0)
    );
}
