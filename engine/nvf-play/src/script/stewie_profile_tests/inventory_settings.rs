//! Existing application choices reach original-style startup caches before execution.
use super::*;

fn assert_queries(rt: &mut Runtime, parent: bool, child: bool, individual: bool) {
    for (key, chosen) in [("bHardcoreTweaks:Tweaks", parent), ("bAmmoWeight:Hardcore Tweaks", child), ("bIndividialItemStats:Tweaks", individual)] {
        for name in ["StewVal", "StewieGetValue"] {
            assert_eq!(source(rt, &format!("SetFunctionValue ({name} \"{key}\")")), XVal::Num(f64::from(u8::from(chosen))), "{name} {key}");
        }
    }
}

#[test]
#[ignore = "process-local roots: run exact selector alone with dedicated NVF_STEWIE_PROFILE_HOME and --test-threads=1"]
fn loaded_inventory_choices_reach_startup_and_restore_without_importing_conflicting_file_values() {
    let files = OwnedInputs::new();
    let original_path = files.data.join("NVSE/Plugins/nvse_stewie_tweaks.ini");
    std::fs::write(&original_path, "[Tweaks]\nbHardcoreTweaks=1\nbIndividialItemStats=1\n[Hardcore Tweaks]\nbAmmoWeight=1\n").unwrap();
    std::fs::write(&files.nvf, "[Settings]\ntweak-hardcore-tweaks=0\ntweak-hardcore-ammo-weight=0\ntweak-single-item-figures=0\n").unwrap();
    let original = std::fs::read(&original_path).unwrap();
    let preferences = std::fs::read(&files.nvf).unwrap();
    let file = SettingsFile(Some(files.nvf.clone()));
    let mut app = App::new();
    app.add_plugins(crate::tweaks::TweaksPlugin);
    app.insert_resource(file.clone());
    crate::settings::apply_stored(app.world_mut());
    let mut settings = app.world().resource::<TweakSettings>().clone();
    assert!(!settings.game.hardcore_tweaks);
    assert!(!settings.game.hardcore_ammo_weight);
    assert!(!settings.game.single_item_figures);
    let body = "SetModLocalData \"inventory.parent\" (StewVal \"bHardcoreTweaks:Tweaks\")\nSetModLocalData \"inventory.child\" (StewieGetValue \"bAmmoWeight:Hardcore Tweaks\")\nSetModLocalData \"inventory.single\" (StewVal \"bIndividialItemStats:Tweaks\")\n";
    for name in ["gr_owned_profile.txt", "ln_owned_profile.txt"] {
        std::fs::write(files.data.join("NVSE/Plugins/scripts").join(name), body).unwrap();
    }
    let (mut rt, mut sim) = fresh(&mut settings, &mut ProfileState::default(), &file);
    assert_queries(&mut rt, false, false, false);
    script_runner(&mut rt, &mut sim, Moment::Restart);
    script_runner(&mut rt, &mut sim, Moment::NewGame);
    for key in ["inventory.parent", "inventory.child", "inventory.single"] {
        assert_eq!(source(&mut rt, &format!("SetFunctionValue (GetModLocalData \"{key}\")")), XVal::Num(0.0), "startup cache {key}");
    }
    let saved = with_world(&mut rt, &mut sim, |rt| rt.game.snapshot());
    settings.game.hardcore_ammo_weight = true;
    settings.game.single_item_figures = true;
    rt.restore_save(&mut sim, saved).unwrap();
    stewie_profile::handoff(&mut rt.game, &settings);
    assert_queries(&mut rt, false, true, true);
    script_runner(&mut rt, &mut sim, Moment::Restart);
    script_runner(&mut rt, &mut sim, Moment::Load);
    for (key, expected) in [("inventory.parent", 0.0), ("inventory.child", 1.0), ("inventory.single", 1.0)] {
        assert_eq!(source(&mut rt, &format!("SetFunctionValue (GetModLocalData \"{key}\")")), XVal::Num(expected), "restored cache {key}");
    }
    rt.game.sync_perk_weights();
    assert_eq!(rt.game.world.weight_rules(fnv_sim::FormId::PLAYER_REF).ammo_weight, None, "retained ammo choice remains gated by disabled parent");
    assert_eq!(std::fs::read(&original_path).unwrap(), original);
    assert_eq!(std::fs::read(&files.nvf).unwrap(), preferences);
}
