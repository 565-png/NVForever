//! Compatibility-profile defaults for existing real critical-hit rules.
use super::TweakSettings;
use crate::settings::{SettingsFile, SECTION};
use bevy::prelude::*;

#[derive(Resource, Default, Debug)]
pub(crate) struct ProfileState {
    initialized: bool,
}
pub(crate) type StartupProfile<'a> = (
    &'a mut TweakSettings,
    &'a mut ProfileState,
    Option<&'a SettingsFile>,
);

/// Called before startup quests/runner files, not one frame after their INI reads.
pub(crate) fn prepare(
    game: &mut fnv_game::Game<'_>,
    settings: &mut TweakSettings,
    state: &mut ProfileState,
    file: Option<&SettingsFile>,
) {
    if !state.initialized {
        state.initialized = true;
        if let Some(profile) = game.stewie_stat_profile() {
            let stored = file
                .and_then(|f| f.0.as_ref())
                .map(std::fs::read_to_string)
                .transpose();
            let text = match stored {
                Ok(text) => text.unwrap_or_default(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
                Err(e) => {
                    warn!("Stewie stat profile not imported: NVForever setting precedence could not be read: {e}");
                    handoff(game, settings);
                    return;
                }
            };
            if let Some(value) = profile.crit_ignores_fire_rate {
                if fnv_install::ini_get(&text, SECTION, "tweak-crit-ignores-fire-rate").is_none() {
                    settings.game.crit_ignores_fire_rate = value;
                    info!("Stewie stat profile: imported bCritChanceIgnoresFireRate={value}");
                } else {
                    info!("Stewie stat profile: retained stored NVForever tweak-crit-ignores-fire-rate (including legacy/default-valued keys); original profile does not override it");
                }
            }
            if let Some(value) = profile.crit_damage_scale {
                if fnv_install::ini_get(&text, SECTION, "tweak-crit-damage-scale").is_none() {
                    settings.game.crit_damage_scale = value;
                    info!("Stewie stat profile: imported critical damage multiplier{value}");
                } else {
                    info!("Stewie stat profile: retained stored NVForever tweak-crit-damage-scale (including legacy/default-valued keys); original profile does not override it");
                }
            }
        }
    }
    handoff(game, settings);
}
/// Settings are application state, not saved game defaults. Restore before Restart.
pub(crate) fn handoff(game: &mut fnv_game::Game<'_>, settings: &TweakSettings) {
    game.bind_stewie_stat_settings(&settings.game);
}
