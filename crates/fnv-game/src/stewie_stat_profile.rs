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
