//! ArchivePlugin: the game's virtual file system and the NIF schema as resources.

use bevy::prelude::*;
use fnv_nif::Schema;
use fnv_vfs::Vfs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

#[derive(Resource, Clone)]
pub struct GameData {
    /// The mounted VFS. Read it freely, but replace it only through [`GameData::set_vfs`]
    /// (or a whole [`GameData::new`]): the lazy authored-material index belongs to the
    /// mount it was built from, and assigning this field directly would leave the previous
    /// mount's index attached. (Public because it is read in many places.)
    pub vfs: Arc<Vfs>,
    pub schema: Arc<Schema>,
    /// The authored material sets under `textures\pbr\` (#2981), built from `vfs` the
    /// first time [`GameData::pbr_index`] is asked; clones share it, as they share `vfs`.
    /// Private, so it is only ever made together with its VFS.
    pbr: Arc<OnceLock<fnv_pbr::PbrIndex>>,
}

impl GameData {
    /// `vfs` and `schema` with an authored-material index of their own, not yet built.
    pub fn new(vfs: Arc<Vfs>, schema: Arc<Schema>) -> GameData {
        GameData { vfs, schema, pbr: Arc::new(OnceLock::new()) }
    }

    /// Mount another VFS: the authored-material index starts afresh with it. Clones made
    /// before keep the old VFS and its index together.
    pub fn set_vfs(&mut self, vfs: Arc<Vfs>) {
        self.vfs = vfs;
        self.pbr = Arc::new(OnceLock::new());
    }

    /// The sets of authored material maps the VFS supplies (docs/spec-notes/pbr-materials.md),
    /// built on first use from the VFS's paths under `textures\pbr\` (no file is read) and
    /// logged in one line. Asked for only while the run's Authored materials setting is on
    /// (`crate::authored`).
    pub fn pbr_index(&self) -> &fnv_pbr::PbrIndex {
        self.pbr.get_or_init(|| {
            let index = fnv_pbr::PbrIndex::build(self.vfs.paths(|k| k.starts_with(fnv_pbr::PBR_DIR)));
            info!("{}", index.summary());
            index
        })
    }

    /// Whether [`GameData::pbr_index`] has been built (the setting off never asks: tests and
    /// the field, `pbr_field.rs`).
    pub(crate) fn pbr_index_built(&self) -> bool {
        self.pbr.get().is_some()
    }

    /// Open the default archive set of `data_dir` (read-only) and load the schemas.
    pub fn open(data_dir: &Path, schema_dir: &Path) -> Result<GameData, String> {
        GameData::open_for_plugins(data_dir, &fnv_vfs::DEFAULT_PLUGINS, schema_dir)
    }

    /// [`GameData::open`] with the archives of `plugins` (load order): mods' archives
    /// over the base game's, loose files over both.
    pub fn open_for_plugins<S: AsRef<str>>(data_dir: &Path, plugins: &[S], schema_dir: &Path) -> Result<GameData, String> {
        GameData::open_for_plugins_in(data_dir, None, plugins, schema_dir)
    }

    /// [`GameData::open_for_plugins`] with the archives beside the plugins in NVForever's
    /// mods folder `mods` too (`fnv_vfs::Vfs::archive_order_in`).
    pub fn open_for_plugins_in<S: AsRef<str>>(data_dir: &Path, mods: Option<&Path>, plugins: &[S], schema_dir: &Path) -> Result<GameData, String> {
        let vfs = Vfs::open_for_plugins_in(data_dir, mods, plugins).map_err(|e| e.to_string())?;
        let schema = Schema::load_dir(schema_dir).map_err(|e| e.to_string())?;
        Ok(GameData::new(Arc::new(vfs), Arc::new(schema)))
    }

    /// [`GameData::open_for_plugins`] with loose files of the mods folders `roots` layered
    /// over `Data`, lowest precedence first (#1526, Mod Organizer 2's profile).
    pub fn open_for_plugins_layered<S: AsRef<str>>(data_dir: &Path, roots: &[PathBuf], plugins: &[S], schema_dir: &Path) -> Result<GameData, String> {
        let vfs = Vfs::open_for_plugins_layered(data_dir, roots, plugins).map_err(|e| e.to_string())?;
        let schema = Schema::load_dir(schema_dir).map_err(|e| e.to_string())?;
        Ok(GameData::new(Arc::new(vfs), Arc::new(schema)))
    }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        let broken = broken_files().iter().find(|(_, part)| path.to_ascii_lowercase().contains(part.as_str()));
        if let Some((Broken::Missing, _)) = broken {
            return None;
        }
        let bytes = match self.vfs.read(path)? {
            Ok(d) => d,
            Err(e) => {
                warn!("{path}: {e}");
                return None;
            }
        };
        test_broken(path, bytes)
    }
}

/// `bytes` of the file at `path` as the test aid [`broken_files`] has them: unchanged,
/// missing, garbled, or a panic. For readers that do not go through [`GameData::read`]
/// (the voice bank).
pub fn test_broken(path: &str, mut bytes: Vec<u8>) -> Option<Vec<u8>> {
    let broken = broken_files().iter().find(|(_, part)| path.to_ascii_lowercase().contains(part.as_str()));
    match broken.map(|b| b.0) {
        Some(Broken::Missing) => return None,
        Some(Broken::Corrupt) => corrupt(&mut bytes),
        Some(Broken::Panic) => panic!("test panic reading {path} (NVF_PLAY_TEST_BROKEN_FILES)"),
        None => {}
    }
    Some(bytes)
}

/// How a file named in the test aid is broken.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Broken {
    /// As if a mod left it out.
    Missing,
    /// Its header kept, the rest garbled.
    Corrupt,
    /// Reading it panics (a bug of ours met by a bad file).
    Panic,
}

/// Test aid `NVF_PLAY_TEST_BROKEN_FILES="missing:<part>;corrupt:<part>;panic:<part>"`:
/// files whose archive path contains `<part>` (any case) read as missing, garbled or
/// panicking, to check the game goes on (guard.rs). A test aid: nothing is changed on
/// disk.
fn broken_files() -> &'static [(Broken, String)] {
    static FILES: std::sync::OnceLock<Vec<(Broken, String)>> = std::sync::OnceLock::new();
    FILES.get_or_init(|| std::env::var("NVF_PLAY_TEST_BROKEN_FILES").map(|v| parse_broken(&v)).unwrap_or_default())
}

fn parse_broken(v: &str) -> Vec<(Broken, String)> {
    v.split(';')
        .filter_map(|e| {
            let (kind, part) = e.trim().split_once(':')?;
            let kind = match kind.trim() {
                "missing" => Broken::Missing,
                "corrupt" => Broken::Corrupt,
                "panic" => Broken::Panic,
                _ => return None,
            };
            let part = part.trim().replace('/', "\\").to_ascii_lowercase();
            (!part.is_empty()).then_some((kind, part))
        })
        .collect()
}

/// Garble every byte after the first 64 (a NIF or DDS header stays readable, so the
/// parsers get past it into the broken part). Fixed, so runs repeat.
fn corrupt(bytes: &mut [u8]) {
    for (i, b) in bytes.iter_mut().enumerate().skip(64) {
        *b ^= (i as u8).wrapping_mul(151).wrapping_add(0x5A);
    }
}

pub struct ArchivePlugin(pub GameData);

impl Plugin for ArchivePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(crate::nif_material::NifMaterialPlugin)
            .insert_resource(self.0.clone())
            .init_resource::<crate::texture::TextureCache>()
            .init_resource::<crate::model::ModelCache>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broken_file_list() {
        assert_eq!(
            parse_broken("missing: Meshes/Clutter/ ; corrupt:mtidle.kf;bogus:x;panic:"),
            vec![(Broken::Missing, "meshes\\clutter\\".to_string()), (Broken::Corrupt, "mtidle.kf".to_string())]
        );
        let mut b = vec![0u8; 70];
        corrupt(&mut b);
        assert!(b[..64].iter().all(|&x| x == 0) && b[64..].iter().any(|&x| x != 0));
    }

    /// The authored-material index is built from the VFS on first use, shared by clones,
    /// and fresh for a `GameData` over another VFS (#2981).
    #[test]
    fn pbr_index_is_lazy_shared_and_per_vfs() {
        let dir = std::env::temp_dir().join(format!("nvf-assets-pbr-index-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let other = dir.join("other");
        for (root, rel) in [(&dir, "textures/pbr/walls/brick07_orm.dds"), (&dir, "textures/pbr/walls/brick07_n.dds"), (&other, "textures/walls/brick07.dds")] {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        }
        let data = GameData::new(Arc::new(Vfs::open(&dir, &[]).unwrap()), Arc::new(Schema::default()));
        assert!(data.pbr.get().is_none(), "built before it was asked for");
        let clone = data.clone();
        assert_eq!(data.pbr_index().counts().authored, 1);
        assert!(clone.pbr.get().is_some(), "a clone shares the built index");
        assert!(data.pbr_index().lookup(r"textures\walls\brick07.dds").is_some());
        let fresh = GameData::new(Arc::new(Vfs::open(&other, &[]).unwrap()), data.schema.clone());
        assert!(fresh.pbr.get().is_none());
        assert!(fresh.pbr_index().is_empty());
        // Mounting another VFS in place renews the index; the clone keeps the old pair.
        let mut moved = data.clone();
        moved.set_vfs(Arc::new(Vfs::open(&other, &[]).unwrap()));
        assert!(moved.pbr.get().is_none());
        assert!(moved.pbr_index().is_empty());
        assert_eq!(clone.pbr_index().counts().authored, 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
