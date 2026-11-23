//! Orthogonal record- and text-backed ySI tag predicates (`docs/spec-notes/ysi-filename.md`).
//!
//! Covers predicates outside the core `mod`/`form`/`formlist`/`formType`/`questItem` set
//! and outside the `WEAP`/`ALCH`/`ARMO` core subset:
//! - Extra `WEAP` traits (`weaponClipRounds`, `weaponNumProjectiles`, `weaponAttackAnim`,
//!   `weaponHandgrip`, `weaponSoundLevel`, `ammoMod` + `ammoForm`)
//! - `ALCH` effect/classification traits (`aidRestoresAV`, `aidDamagesAV`, `aidIsWater`,
//!   `aidIsPoisonous`)
//! - `BOOK` (`bookSkill`, `isSkillBook`), `NOTE` (`noteType`), `AMMO` (`ammoIgnoresDTDR`)
//! - Recipe membership (`miscComponent`, `miscProduct`)
//! - Name / EditorID text matching (`prefix`, `suffix`, `edid`, `edidPrefix`)
//!
//! All mappings are `[hypothesis]` and read existing `Game` / `GameData` / `FormStore` views;
//! mismatched record types disqualify as `Match::No` while malformed or unsupported values
//! remain `Match::Unknown`.

use crate::aid::{self, value_change};
use crate::data::MagicEffectDef;
use crate::game::Game;
use crate::ysi_filename::{any, number_set, parse_local_form, Match};
use fnv_script::vm::XVal;
use fnv_sim::FormId;
use serde_json::{Map, Value as Json};

/// Evaluates an extended ySI predicate key. Returns `Match::Unknown` for unrecognized keys
/// or malformed predicate values.
pub(crate) fn eval(
    g: &Game<'_>,
    item: FormId,
    fields: &Map<String, Json>,
    name: &str,
    value: &Json,
) -> Match {
    match name {
        // Companion selector evaluated together with `ammoForm`.
        "ammoMod" => Match::Yes,
        "ammoForm" => weapon_ammo_match(g, item, fields, value),
        "weaponClipRounds" => weapon_int_field(g, item, "DATA", "clip_size", value),
        "weaponNumProjectiles" => weapon_int_field(g, item, "DNAM", "projectile_count", value),
        "weaponAttackAnim" => weapon_int_field(g, item, "DNAM", "attack_animation", value),
        "weaponHandgrip" => weapon_int_field(g, item, "DNAM", "grip_animation", value),
        "weaponSoundLevel" => weapon_int_field(g, item, "VNAM", "", value),
        "aidRestoresAV" => aid_effect_av(g, item, value, true),
        "aidDamagesAV" => aid_effect_av(g, item, value, false),
        "aidIsWater" => aid_is_water(g, item, value),
        "aidIsPoisonous" => aid_is_poisonous(g, item, value),
        "bookSkill" => book_skill(g, item, value),
        "isSkillBook" => is_skill_book(g, item, value),
        "noteType" => note_type(g, item, value),
        "ammoIgnoresDTDR" => ammo_ignores_dtdr(g, item, value),
        "miscComponent" => recipe_membership(g, item, value, true),
        "miscProduct" => recipe_membership(g, item, value, false),
        "prefix" => name_match(g, item, value, TextMatch::Prefix),
        "suffix" => name_match(g, item, value, TextMatch::Suffix),
        "edid" => edid_match(g, item, value, TextMatch::Exact),
        "edidPrefix" => edid_match(g, item, value, TextMatch::Prefix),
        _ => Match::Unknown,
    }
}

/// Returns `true` when a companion selector (`ammoMod`) appears without its target (`ammoForm`).
pub(crate) fn has_dangling_companions(fields: &Map<String, Json>) -> bool {
    fields.contains_key("ammoMod") && !fields.contains_key("ammoForm")
}

/// Match a `0`/`1` or `-1`/`1` boolean ySI predicate (`1` = true, `0`/`-1` = false;
/// other values remain `Match::Unknown`) [hypothesis].
fn bool_flag(value: &Json, actual: bool) -> Match {
    match value.as_i64() {
        Some(1) => if actual { Match::Yes } else { Match::No },
        Some(0 | -1) => if !actual { Match::Yes } else { Match::No },
        _ => Match::Unknown,
    }
}

fn weapon_int_field(g: &Game<'_>, item: FormId, sub: &str, path: &str, value: &Json) -> Match {
    if g.data.sig(item) != Some(*b"WEAP") {
        return Match::No;
    }
    match crate::nvse::record_value(g, item, sub, path) {
        Some(XVal::Num(n)) if n.is_finite() => number_set(value, n.trunc() as i64),
        _ => Match::Unknown,
    }
}

fn weapon_ammo_match(g: &Game<'_>, item: FormId, fields: &Map<String, Json>, value: &Json) -> Match {
    if g.data.sig(item) != Some(*b"WEAP") {
        return Match::No;
    }
    let Some(plugin) = fields.get("ammoMod").and_then(Json::as_str) else {
        return Match::Unknown;
    };
    let Some(index) = g
        .data
        .load_order
        .iter()
        .position(|p| p.eq_ignore_ascii_case(plugin))
    else {
        return Match::No;
    };
    if index > 255 {
        return Match::Unknown;
    }
    let ammo = g.weapon_ammo(item);
    let check = |v: &Json| {
        let Some(local) = parse_local_form(v) else {
            return Match::Unknown;
        };
        let target = FormId(((index as u32) << 24) | local);
        if ammo.contains(&target) {
            Match::Yes
        } else {
            Match::No
        }
    };
    match value {
        Json::Array(values) => any(values.iter().map(check)),
        _ => check(value),
    }
}

fn aid_effect_av(g: &Game<'_>, item: FormId, value: &Json, restores: bool) -> Match {
    if g.data.sig(item) != Some(*b"ALCH") {
        return Match::No;
    }
    let base = g.unclone(item);
    let effects = g.data.effect_items.get(&base).map(Vec::as_slice).unwrap_or(&[]);
    let matches_code = |code: i64| {
        effects.iter().any(|e| {
            let Some(m) = g.magic_effect_def(e.effect) else {
                return false;
            };
            let Some(change) = value_change(&m, e, 1.0, false) else {
                return false;
            };
            if i64::from(change.code) != code {
                return false;
            }
            let detrimental = m.has(MagicEffectDef::DETRIMENTAL);
            if restores {
                !detrimental && change.amount > 0.0
            } else {
                detrimental || change.amount < 0.0
            }
        })
    };
    let one = |v: &Json| match v.as_i64() {
        Some(code) => if matches_code(code) { Match::Yes } else { Match::No },
        None => Match::Unknown,
    };
    match value {
        Json::Array(values) => any(values.iter().map(one)),
        _ => one(value),
    }
}

fn aid_is_water(g: &Game<'_>, item: FormId, value: &Json) -> Match {
    if g.data.sig(item) != Some(*b"ALCH") {
        return Match::No;
    }
    let base = g.unclone(item);
    let restores_dehydration = g
        .data
        .effect_items
        .get(&base)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .any(|e| {
            let Some(m) = g.magic_effect_def(e.effect) else {
                return false;
            };
            value_change(&m, e, 1.0, false).is_some_and(|c| {
                c.code == aid::DEHYDRATION && !m.has(MagicEffectDef::DETRIMENTAL) && c.amount > 0.0
            })
        });
    bool_flag(value, restores_dehydration)
}

fn aid_is_poisonous(g: &Game<'_>, item: FormId, value: &Json) -> Match {
    if g.data.sig(item) != Some(*b"ALCH") {
        return Match::No;
    }
    bool_flag(value, g.is_poison(item))
}

fn book_skill_code(g: &Game<'_>, item: FormId) -> Option<i64> {
    let base = g.unclone(item);
    if let Some(XVal::Num(n)) = g
        .field_overrides
        .get(&(item, "DATA/skill_code".into()))
        .or_else(|| g.field_overrides.get(&(base, "DATA/skill_code".into())))
    {
        return n.is_finite().then_some(n.trunc() as i64);
    }
    match crate::nvse::record_value(g, item, "DATA", "skill") {
        Some(XVal::Num(n)) if n.is_finite() => {
            let i = n.trunc() as i64;
            Some(if i < 0 { -1 } else { i + 32 })
        }
        _ => None,
    }
}

fn book_skill(g: &Game<'_>, item: FormId, value: &Json) -> Match {
    if g.data.sig(item) != Some(*b"BOOK") {
        return Match::No;
    }
    match book_skill_code(g, item) {
        Some(code) => number_set(value, code),
        None => Match::Unknown,
    }
}

fn is_skill_book(g: &Game<'_>, item: FormId, value: &Json) -> Match {
    if g.data.sig(item) != Some(*b"BOOK") {
        return Match::No;
    }
    let teaches = match book_skill_code(g, item) {
        Some(code) => code >= 0,
        None => g.world.forms.item(item).is_some_and(|i| i.teaches.is_some()),
    };
    bool_flag(value, teaches)
}

fn note_type(g: &Game<'_>, item: FormId, value: &Json) -> Match {
    if g.data.sig(item) != Some(*b"NOTE") {
        return Match::No;
    }
    let base = g.unclone(item);
    let kind = g.data.notes.get(&base).map(|n| i64::from(n.kind)).or_else(|| {
        match crate::nvse::record_value(g, item, "DATA", "") {
            Some(XVal::Num(n)) if n.is_finite() => Some(n.trunc() as i64),
            _ => None,
        }
    });
    match kind {
        Some(k) => number_set(value, k),
        None => Match::Unknown,
    }
}

fn ammo_ignores_dtdr(g: &Game<'_>, item: FormId, value: &Json) -> Match {
    if g.data.sig(item) != Some(*b"AMMO") {
        return Match::No;
    }
    let flags = match crate::nvse::record_value(g, item, "DATA", "flags") {
        Some(XVal::Num(n)) if n.is_finite() => n.trunc() as i64 as u64,
        _ => return Match::Unknown,
    };
    bool_flag(value, flags & 1 != 0)
}

fn recipe_membership(g: &Game<'_>, item: FormId, value: &Json, ingredient: bool) -> Match {
    if g.world.forms.item(g.unclone(item)).is_none() {
        return Match::No;
    }
    let target = g.unclone(item);
    let has = g.data.recipes.keys().copied().any(|id| {
        let recipe = g
            .other
            .lutana
            .recipes
            .get(&id)
            .or_else(|| g.data.recipes.get(&id));
        let Some(r) = recipe else { return false };
        let list = if ingredient { &r.ingredients } else { &r.outputs };
        list.iter().any(|&(f, _)| g.unclone(f) == target)
    });
    bool_flag(value, has)
}

#[derive(Clone, Copy)]
enum TextMatch {
    Exact,
    Prefix,
    Suffix,
}

fn text_match(value: &Json, actual: &str, mode: TextMatch) -> Match {
    let actual_lower = actual.to_ascii_lowercase();
    let one = |v: &Json| {
        let Some(needle) = v.as_str() else {
            return Match::Unknown;
        };
        let needle_lower = needle.to_ascii_lowercase();
        let hit = match mode {
            TextMatch::Exact => actual_lower == needle_lower,
            TextMatch::Prefix => actual_lower.starts_with(&needle_lower),
            TextMatch::Suffix => actual_lower.ends_with(&needle_lower),
        };
        if hit {
            Match::Yes
        } else {
            Match::No
        }
    };
    match value {
        Json::Array(values) => any(values.iter().map(one)),
        _ => one(value),
    }
}

fn name_match(g: &Game<'_>, item: FormId, value: &Json, mode: TextMatch) -> Match {
    let base = g.unclone(item);
    let display = g
        .world
        .forms
        .get(base)
        .and_then(|b| b.name.as_deref())
        .or_else(|| g.data.record_field(base, "FULL", "").and_then(Json::as_str))
        .unwrap_or("");
    text_match(value, display, mode)
}

fn edid_match(g: &Game<'_>, item: FormId, value: &Json, mode: TextMatch) -> Match {
    let base = g.unclone(item);
    let edid = g
        .world
        .forms
        .get(base)
        .and_then(|b| b.edid.as_deref())
        .or_else(|| g.data.edid_of.get(&base).map(String::as_str))
        .unwrap_or("");
    text_match(value, edid, mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_flag_and_text_match_cover_valid_and_malformed_inputs() {
        assert_eq!(bool_flag(&Json::from(1), true), Match::Yes);
        assert_eq!(bool_flag(&Json::from(1), false), Match::No);
        assert_eq!(bool_flag(&Json::from(0), false), Match::Yes);
        assert_eq!(bool_flag(&Json::from(-1), false), Match::Yes);
        assert_eq!(bool_flag(&Json::from(-1), true), Match::No);
        assert_eq!(bool_flag(&Json::from(2), false), Match::Unknown);
        assert_eq!(bool_flag(&Json::from("1"), true), Match::Unknown);

        assert_eq!(text_match(&Json::from("Nuka-"), "Nuka-Cola Quantum", TextMatch::Prefix), Match::Yes);
        assert_eq!(text_match(&Json::from("Sunset"), "Nuka-Cola Quantum", TextMatch::Prefix), Match::No);
        assert_eq!(text_match(&Json::from("quantum"), "Nuka-Cola Quantum", TextMatch::Suffix), Match::Yes);
        assert_eq!(
            text_match(&serde_json::json!(["Foo", "nuka-"]), "Nuka-Cola", TextMatch::Prefix),
            Match::Yes
        );
        assert_eq!(
            text_match(&serde_json::json!(["Foo", 42]), "Nuka-Cola", TextMatch::Prefix),
            Match::Unknown
        );
        assert_eq!(
            text_match(&Json::from("Weap10mm"), "weap10mm", TextMatch::Exact),
            Match::Yes
        );
    }

    #[test]
    fn dangling_ammo_mod_without_ammo_form_is_flagged() {
        let mut only_mod = Map::new();
        only_mod.insert("ammoMod".into(), Json::from("FalloutNV.esm"));
        assert!(has_dangling_companions(&only_mod));

        only_mod.insert("ammoForm".into(), Json::from("00004241"));
        assert!(!has_dangling_companions(&only_mod));
    }
}
