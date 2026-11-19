//! Authored PBR material sets as materials use them (#2981, docs/spec-notes/pbr-materials.md).
//!
//! - The run's switch ([`enabled`]): the settings' Authored materials row, read once at the
//!   start ([`set_enabled`], nvforever `main.rs`) and fixed for the run, so cached models
//!   never mix the two states. Off, nothing here asks for the index (`GameData::pbr_index`)
//!   or reads a file under `textures\pbr\`.
//! - [`find`]: a shape's set by its resolved diffuse key, the members loaded by the exact
//!   paths the index names, the scalar file read once per set ([`Memo`]).
//!
//! [design] The rules are NVForever's own (the format is ours); none describes the original.

use crate::data::GameData;
use crate::texture::TextureUsage;
use bevy::prelude::*;
use fnv_pbr::{Found, Member, PbrIndex, Scalars, SetState};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

/// The run's state of the Authored materials setting (module notes).
static RUN: OnceLock<bool> = OnceLock::new();

/// Fix the run's state of the setting, before the first material is made. Called once at the
/// start; a later call that disagrees is refused with a warning (the state is the run's).
pub fn set_enabled(on: bool) {
    if RUN.set(on).is_err() && enabled() != on {
        warn!("authored materials: already {} for this run; the setting takes effect at the next start", if enabled() { "on" } else { "off" });
    }
}

/// Whether authored sets shape materials in this run: as [`set_enabled`] fixed it, else on
/// (the default; the viewer never sets it).
pub fn enabled() -> bool {
    *RUN.get_or_init(|| true)
}

/// An authored set found for a shape's diffuse, its maps loaded.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthoredSet {
    /// The set's stem (`textures\pbr\...`, without suffix).
    pub stem: String,
    /// `_orm`: occlusion red, roughness green, metallic blue (linear).
    pub orm: Handle<Image>,
    /// `_n` in the two-channel role, when the set has one and it loaded.
    pub normal: Option<Handle<Image>>,
    /// The scalar file's values (all `None` without one).
    pub scalars: Scalars,
}

/// What [`find`] remembers per mount: the scalar files by path, and the files already warned
/// about (one warning per file).
#[derive(Default)]
pub struct Memo {
    scalars: Mutex<HashMap<String, Scalars>>,
    warned: Mutex<HashSet<String>>,
}

impl Memo {
    /// Log `message` the first time `path` is warned about.
    pub(crate) fn warn_once(&self, path: &str, message: impl FnOnce() -> String) {
        if self.warned.lock().expect("authored warnings").insert(path.to_string()) {
            warn!("{}", message());
        }
    }

    /// The scalars of the file at `path`, parsed the first time (its warnings logged then).
    fn scalars(&self, path: &str, read: impl FnOnce() -> Option<Vec<u8>>) -> Scalars {
        if let Some(s) = self.scalars.lock().expect("authored scalars").get(path) {
            return *s;
        }
        let scalars = match read() {
            Some(bytes) => {
                let file = fnv_pbr::parse_scalars(&bytes);
                for w in &file.warnings {
                    warn!("authored materials: {path}: {w}");
                }
                file.scalars
            }
            None => {
                warn!("authored materials: {path} could not be read; the defaults apply");
                Scalars::default()
            }
        };
        self.scalars.lock().expect("authored scalars").insert(path.to_string(), scalars);
        scalars
    }
}

/// The authored set (state [`SetState::Authored`]) for the resolved diffuse `key`. A
/// height-only set changes no material in this stage: one for the key itself is no set, and
/// one for a first-person twin gives way to the plain texture's set, as the twin's absence
/// would [design: stage 3 of #2981 draws no height maps].
pub fn authored_found(index: &PbrIndex, key: &str) -> Option<Found> {
    let found = index.lookup(key)?;
    if found.state() == SetState::Authored {
        return Some(found);
    }
    if found.first_person_fallback || !crate::texture::is_first_person_texture(key) {
        return None;
    }
    // The twin's own set is height only: try the plain name.
    let path = fnv_vfs::texture_path(key);
    let file = path.rfind('\\').map_or(0, |i| i + 1);
    let plain = format!("{}{}", &path[..file], path[file..].strip_prefix("1stperson")?);
    let plain_found = index.lookup(&plain).filter(|f| f.state() == SetState::Authored)?;
    Some(Found { first_person_fallback: true, ..plain_found })
}

/// The authored set for the resolved diffuse `key` with its maps loaded by `load` (path as the
/// index names it, exactly: never resolved again, so a `\xbox\` member stays the one listed;
/// role), or `None`: the setting off (`enabled`; nothing is asked of the index then), no
/// authored set, or an `_orm` that does not load (one warning per file; the shape keeps the
/// game's material). An `_n` that does not load leaves the game's normal map in use.
pub(crate) fn find(data: &GameData, enabled: bool, key: &str, memo: &Memo, mut load: impl FnMut(&str, TextureUsage) -> Option<Handle<Image>>) -> Option<AuthoredSet> {
    if !enabled {
        return None;
    }
    let found = authored_found(data.pbr_index(), key)?;
    let orm_path = found.path(Member::Orm)?;
    let Some(orm) = load(&orm_path, TextureUsage::Linear) else {
        memo.warn_once(&orm_path, || format!("authored materials: {orm_path} could not be loaded; surfaces drawn with {key} keep the game's material"));
        return None;
    };
    let normal = found.path(Member::Normal).and_then(|path| {
        let h = load(&path, TextureUsage::NormalTwoChannel);
        if h.is_none() {
            memo.warn_once(&path, || format!("authored materials: {path} could not be loaded; the game's normal map is used"));
        }
        h
    });
    let scalars = found.path(Member::Scalars).map(|path| memo.scalars(&path, || data.read(&path))).unwrap_or_default();
    Some(AuthoredSet { stem: found.stem, orm, normal, scalars })
}

/// The warning for a linear member whose DX10 header names an `_SRGB` format (once per file:
/// the decode of a (path, role) happens once).
pub(crate) fn srgb_warning(memo: &Memo, path: &str, usage: TextureUsage, srgb_code: bool) {
    if srgb_code && usage != TextureUsage::Color && path.starts_with(fnv_pbr::PBR_DIR) {
        memo.warn_once(path, || format!("authored materials: {path} names an sRGB format in its DX10 header; it is read as linear data, as its role is"));
    }
}

#[cfg(test)]
pub(crate) mod tests;
