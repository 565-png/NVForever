//! Typed supported stat-profile inputs; no provider/version advertisement.
use crate::Game;

/// Supported original-profile inputs for existing critical-hit consumers.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct StewieStatProfile {
    pub crit_ignores_fire_rate: Option<bool>,
    pub crit_damage_scale: Option<f64>,
}
impl StewieStatProfile {
    /// Read only the documented supported subset. Missing/malformed values never
    /// enable a setting; scaling is bounded by the existing engine slider's range.
    pub fn parse(text: &str) -> Self {
        let read = |section: &str, key: &str| {
            crate::mod_files::ini_get(text, &section.to_ascii_lowercase(), key)
        };
        let boolean = |key: &str| match read("Tweaks", key)?.trim() {
            "0" => Some(false),
            "1" => Some(true),
            _ => None,
        };
        let crit_damage_scale = match boolean("bScaleCriticalDamage") {
            Some(false) => Some(1.0),
            Some(true) => read("Critical Hits", "fCriticalDamageMult")
                .and_then(|v| v.trim().parse::<f64>().ok())
                .filter(|v| v.is_finite() && (0.0..=2.0).contains(v)),
            None => None,
        };
        Self {
            crit_ignores_fire_rate: boolean("bCritChanceIgnoresFireRate"),
            crit_damage_scale,
        }
    }
}
impl Game<'_> {
    /// Bind application settings before startup/restore scripts. Queries read the
    /// same current fields as combat, with no second copy or saved authority.
    pub fn bind_stewie_stat_settings(&mut self, settings: &crate::tweaks::Tweaks) {
        self.tweaks = settings.clone();
        self.small.stewie_stat_settings_loaded = true;
    }

    pub(crate) fn loaded_stewie_stat_value(&self, key: &str, section: &str) -> Option<f64> {
        if !self.small.stewie_stat_settings_loaded {
            return None;
        }
        if section.trim().eq_ignore_ascii_case("Tweaks")
            && key
                .trim()
                .eq_ignore_ascii_case("bCritChanceIgnoresFireRate")
        {
            return Some(f64::from(u8::from(self.tweaks.crit_ignores_fire_rate)));
        }
        if section.trim().eq_ignore_ascii_case("Critical Hits")
            && key.trim().eq_ignore_ascii_case("fCriticalDamageMult")
        {
            return Some(self.tweaks.crit_damage_scale);
        }
        // There is no separate loaded scale-enable intent in the engine slider.
        // Do not infer that native switch from an effective multiplier of one.
        None
    }

    /// The current layered original-profile text, as the existing StewVal reads it,
    /// reduced to supported typed inputs. Callers decide startup/settings precedence.
    pub fn stewie_stat_profile(&self) -> Option<StewieStatProfile> {
        self.mod_files
            .read_text("NVSE\\Plugins\\nvse_stewie_tweaks.ini")
            .map(|text| StewieStatProfile::parse(&text))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supported_keys_are_typed_and_the_scale_switch_controls_its_existing_range() {
        let p = StewieStatProfile::parse("[Tweaks]\nbCritChanceIgnoresFireRate=1\nbScaleCriticalDamage=1\n[Critical Hits]\nfCriticalDamageMult=1.75\n");
        assert_eq!(p.crit_ignores_fire_rate, Some(true));
        assert_eq!(p.crit_damage_scale, Some(1.75));
        let p = StewieStatProfile::parse("[tWeAkS]\nBCritChanceIgnoresFireRate=0\nbScaleCriticalDamage=0\n[Critical Hits]\nfCriticalDamageMult=99\n");
        assert_eq!(p.crit_ignores_fire_rate, Some(false));
        assert_eq!(p.crit_damage_scale, Some(1.0));
    }
    #[test]
    fn missing_malformed_and_unsupported_inputs_do_not_invent_profile_values() {
        assert_eq!(StewieStatProfile::parse(""), StewieStatProfile::default());
        for flag in ["2", "-1", "true", "NaN"] {
            assert_eq!(
                StewieStatProfile::parse(&format!(
                    "[Tweaks]\nbCritChanceIgnoresFireRate={flag}\nbScaleCriticalDamage={flag}\n"
                )),
                StewieStatProfile::default()
            );
        }
        for scale in ["-0.01", "2.01", "NaN", "inf", "oops"] {
            assert_eq!(StewieStatProfile::parse(&format!("[Tweaks]\nbScaleCriticalDamage=1\n[Critical Hits]\nfCriticalDamageMult={scale}\n")).crit_damage_scale, None);
        }
    }
}

#[cfg(test)]
mod loaded_tests {
    use super::*;
    use crate::{game::GameConfig, GameData};
    use fnv_esm::{LoadOrder, Plugin, Schema};
    use fnv_script::{vm::XVal, FunctionTable};
    use fnv_sim::{FormStore, RefTable, SimConfig, World};

    #[test]
    fn loaded_aliases_read_current_consumer_fields_without_claiming_unrepresented_switches() {
        let bytes = crate::common::tes4();
        let plugins = [Plugin::parse(&bytes, "OwnedLoadedSettings.esm".into()).unwrap()];
        let lo = LoadOrder::new(&plugins).unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas");
        let schema = Schema::load_dir(root.join("fnv")).unwrap();
        let path = root.join("script/functions.json");
        let catalogue = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let forms = FormStore::build(&lo, &schema);
        let refs = RefTable::build(&lo, &schema);
        let data = GameData::build(
            &lo,
            &schema,
            FunctionTable::load(&path).unwrap(),
            &catalogue,
            &refs,
        );
        let mut game = Game::new(
            &data,
            World::new(&forms, &refs, SimConfig::default()),
            GameConfig::default(),
        );
        assert_eq!(
            game.loaded_stewie_stat_value("bCritChanceIgnoresFireRate", "Tweaks"),
            None,
            "headless/default games retain the original query route"
        );
        let mut settings = crate::tweaks::Tweaks::default();
        settings.crit_ignores_fire_rate = true;
        settings.crit_damage_scale = 1.5;
        game.bind_stewie_stat_settings(&settings);
        let query = |g: &mut Game<'_>, name: &str, key: &str| {
            g.run_query(
                &format!("Begin Function {{}}\nSetFunctionValue ({name} \"{key}\")\nEnd\n"),
                Default::default(),
            )
            .unwrap()
        };
        assert_eq!(
            query(
                &mut game,
                "StewVal",
                " bCritChanceIgnoresFireRate : tWEAKs "
            ),
            XVal::Num(1.0)
        );
        assert_eq!(
            query(
                &mut game,
                "StewieGetValue",
                "fCriticalDamageMult:Critical Hits"
            ),
            XVal::Num(1.5)
        );
        // Existing frame sync changes these fields directly; queries must not keep
        // a duplicate startup copy of their former values.
        game.tweaks.crit_ignores_fire_rate = false;
        game.tweaks.crit_damage_scale = 0.0;
        assert_eq!(
            query(
                &mut game,
                "StewieGetValue",
                "bCritChanceIgnoresFireRate:Tweaks"
            ),
            XVal::Num(0.0)
        );
        assert_eq!(
            query(&mut game, "StewVal", "fCriticalDamageMult:Critical Hits"),
            XVal::Num(0.0)
        );
        for (key, section) in [
            ("bScaleCriticalDamage", "Tweaks"),
            ("bMenuSearch", "Tweaks"),
            ("bCritChanceIgnoresFireRate", "Other"),
        ] {
            assert_eq!(game.loaded_stewie_stat_value(key, section), None);
        }
        assert_eq!(query(&mut game, "StewVal", "malformed"), XVal::Num(0.0));
        let snapshot = game.snapshot();
        let mut restored = Game::restore(
            &data,
            World::new(&forms, &refs, SimConfig::default()),
            GameConfig::default(),
            &snapshot,
        )
        .unwrap();
        assert_eq!(
            restored.loaded_stewie_stat_value("fCriticalDamageMult", "Critical Hits"),
            None,
            "loaded application authority is not save-baked"
        );
        restored.bind_stewie_stat_settings(&settings);
        assert_eq!(
            query(
                &mut restored,
                "StewVal",
                "fCriticalDamageMult:Critical Hits"
            ),
            XVal::Num(1.5)
        );
    }
}
