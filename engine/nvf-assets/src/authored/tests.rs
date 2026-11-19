//! Authored set lookup (#2981). Every file is generated here, in a folder of the test's own.

use super::*;
use crate::nif_spawn::{StoreTextures, TextureSource};
use crate::texture::TextureCache;
use bevy::render::render_resource::TextureFormat;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// A legacy DDS file (FourCC `tag`) of one level.
pub(crate) fn legacy(tag: &[u8; 4], w: u32, h: u32, payload: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[..4].copy_from_slice(b"DDS ");
    for (at, v) in [(4, 124u32), (8, 0x1007), (12, h), (16, w), (28, 1), (76, 32), (80, 4), (108, 0x1000)] {
        b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    b[84..88].copy_from_slice(tag);
    b.extend_from_slice(payload);
    b
}

/// A DX10 DDS file (DXGI code `dxgi`, 2D, one array element) of one level.
pub(crate) fn dx10(dxgi: u32, w: u32, h: u32, payload: &[u8]) -> Vec<u8> {
    let mut b = legacy(b"DX10", w, h, &[]);
    for v in [dxgi, 3, 0, 1, 0] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(payload);
    b
}

/// A load order of generated files with authored sets (module notes of the cases below).
pub(crate) struct Pack {
    root: PathBuf,
    pub(crate) data: GameData,
}

impl Pack {
    pub(crate) fn new() -> Pack {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("nvf-authored-{}-{stamp}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let dxt1 = legacy(b"DXT1", 4, 4, &[0xFF, 0xFF, 0, 0, 0, 0, 0, 0]);
        let files: Vec<(&str, Vec<u8>)> = vec![
            // A full set: an 8x8 BC7 `_orm` whose header says sRGB, an 8x4 BC5 `_n`, scalars.
            ("textures/walls/brick.dds", dxt1.clone()),
            ("textures/pbr/walls/brick_orm.dds", dx10(99, 8, 8, &[0; 64])),
            ("textures/pbr/walls/brick_n.dds", legacy(b"ATI2", 8, 4, &[0; 32])),
            ("textures/pbr/walls/brick.toml", b"roughness_scale = 0.5\nreflectance = 0.25\n".to_vec()),
            // `_orm` alone.
            ("textures/walls/nonormal.dds", dxt1.clone()),
            ("textures/pbr/walls/nonormal_orm.dds", dx10(98, 4, 4, &[0; 16])),
            // Height only: no authored set.
            ("textures/walls/stone.dds", dxt1.clone()),
            ("textures/pbr/walls/stone_p.dds", legacy(b"BC4U", 4, 4, &[0; 8])),
            // A first-person twin whose own set is height only, and the plain texture's set.
            ("textures/weapons/gun.dds", dxt1.clone()),
            ("textures/weapons/1stpersongun.dds", dxt1.clone()),
            ("textures/pbr/weapons/gun_orm.dds", dx10(98, 4, 4, &[0; 16])),
            ("textures/pbr/weapons/1stpersongun_p.dds", legacy(b"BC4U", 4, 4, &[0; 8])),
            // An `_orm` that is no DDS file.
            ("textures/walls/bad.dds", dxt1.clone()),
            ("textures/pbr/walls/bad_orm.dds", b"not a dds".to_vec()),
            // A console folder: the member under `\xbox\` and another set's under `\pc\`.
            ("textures/walls/xbox/tile.dds", dxt1.clone()),
            ("textures/pbr/walls/xbox/tile_orm.dds", dx10(98, 4, 4, &[0; 16])),
            ("textures/pbr/walls/pc/tile_orm.dds", dxt1.clone()),
            // No set at all.
            ("textures/walls/plain.dds", dxt1),
        ];
        for (rel, bytes) in files {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, bytes).unwrap();
        }
        let data = GameData::new(Arc::new(fnv_vfs::Vfs::open(&root, &[]).unwrap()), Arc::new(fnv_nif::Schema::default()));
        Pack { root, data }
    }
}

impl Drop for Pack {
    fn drop(&mut self) {
        // The folder is this test's own (a fresh name under the temp folder).
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn the_synchronous_source_loads_a_set_by_its_exact_member_paths() {
    let pack = Pack::new();
    let mut cache = TextureCache::default();
    let mut images = Assets::<Image>::default();
    let mut source = StoreTextures { textures: &mut cache, data: &pack.data, images: &mut images, formats: None };
    let key = source.resolve("Walls/Brick.dds");
    assert_eq!(key, r"textures\walls\brick.dds");
    let set = source.authored(&key).expect("authored set");
    assert_eq!(set.stem, r"textures\pbr\walls\brick");
    assert_eq!(set.scalars, Scalars { parallax_depth: None, roughness_scale: Some(0.5), reflectance: Some(0.25) });
    let normal = set.normal.clone().expect("_n");
    let none = source.authored(&source.resolve("walls/nonormal.dds")).expect("set without _n");
    assert!(none.normal.is_none());
    assert_eq!(none.scalars, Scalars::default());
    assert!(source.authored(&source.resolve("walls/stone.dds")).is_none(), "height only is no authored set");
    assert!(source.authored(&source.resolve("walls/plain.dds")).is_none());
    assert!(source.authored(&source.resolve("walls/bad.dds")).is_none(), "an _orm that does not load leaves the shape legacy");
    let twin = source.authored(&source.resolve("weapons/1stpersongun.dds")).expect("the plain set behind a height-only twin");
    assert_eq!(twin.stem, r"textures\pbr\weapons\gun");
    // The console folder: the member the index names, not the PC file `get` would choose.
    let xbox_key = source.resolve(r"walls\xbox\tile.dds");
    assert_eq!(xbox_key, r"textures\walls\xbox\tile.dds", "no PC diffuse: the key stays");
    let xbox = source.authored(&xbox_key).expect("xbox set");
    let resolved = source.texture(r"textures\pbr\walls\xbox\tile_orm.dds", false).unwrap();
    // Every member is in place and in its role.
    let (orm, xbox_orm) = (set.orm.clone(), xbox.orm.clone());
    let warned = cache.workers.authored.warned.lock().unwrap().clone();
    assert_eq!(images.get(&orm).unwrap().texture_descriptor.format, TextureFormat::Bc7RgbaUnorm, "linear, whatever the header says");
    assert_eq!(images.get(&orm).unwrap().size(), UVec2::new(8, 8));
    assert_eq!(images.get(&normal).unwrap().texture_descriptor.format, TextureFormat::Bc5RgUnorm);
    assert_eq!(images.get(&normal).unwrap().size(), UVec2::new(8, 4));
    assert_eq!(images.get(&xbox_orm).unwrap().texture_descriptor.format, TextureFormat::Bc7RgbaUnorm, "the \\xbox\\ member, exactly");
    assert_eq!(images.get(&resolved).unwrap().texture_descriptor.format, TextureFormat::Bc1RgbaUnorm, "the resolving load picks the PC file");
    assert!(warned.contains(r"textures\pbr\walls\brick_orm.dds"), "the _SRGB code is warned about: {warned:?}");
    assert!(warned.contains(r"textures\pbr\walls\bad_orm.dds"), "{warned:?}");
    assert!(!warned.contains(r"textures\pbr\walls\nonormal_orm.dds"));
}

#[test]
fn off_asks_nothing_of_the_index() {
    let pack = Pack::new();
    let memo = Memo::default();
    assert!(find(&pack.data, false, r"textures\walls\brick.dds", &memo, |_, _| panic!("no file is loaded when off")).is_none());
    assert!(!pack.data.pbr_index_built(), "the index is never built while the setting is off");
    assert!(find(&pack.data, true, r"textures\walls\plain.dds", &memo, |_, _| panic!("no set")).is_none());
    assert!(pack.data.pbr_index_built());
}

#[test]
fn a_scalar_file_is_parsed_once_per_file() {
    let memo = Memo::default();
    let reads = AtomicU64::new(0);
    let read = || {
        reads.fetch_add(1, Ordering::Relaxed);
        Some(b"reflectance = 0.1\nbogus = 1\n".to_vec())
    };
    let a = memo.scalars("textures\\pbr\\a.toml", read);
    let b = memo.scalars("textures\\pbr\\a.toml", || panic!("read again"));
    assert_eq!((a, a.reflectance), (b, Some(0.1)));
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    assert_eq!(memo.scalars("textures\\pbr\\gone.toml", || None), Scalars::default());
}

#[test]
fn a_height_only_set_is_no_authored_set_unless_a_plain_one_stands_behind_a_twin() {
    let index = PbrIndex::build([r"textures\pbr\a\stone_p.dds", r"textures\pbr\a\1stpersongun_p.dds", r"textures\pbr\a\gun_orm.dds", r"textures\pbr\a\1stpersonrifle_p.dds"]);
    assert!(authored_found(&index, r"textures\a\stone.dds").is_none());
    let twin = authored_found(&index, r"textures\a\1stpersongun.dds").unwrap();
    assert_eq!((twin.stem.as_str(), twin.first_person_fallback), (r"textures\pbr\a\gun", true));
    assert!(authored_found(&index, r"textures\a\1stpersonrifle.dds").is_none(), "no plain set behind the twin");
    assert!(authored_found(&index, r"textures\a\gun.dds").is_some());
}
