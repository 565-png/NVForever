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
            // An authored set with `_p` and a depth; a height-only set of depth 0; an
            // authored set whose `_p` is no DDS file.
            ("textures/walls/relief.dds", dxt1.clone()),
            ("textures/pbr/walls/relief_orm.dds", dx10(98, 4, 4, &[0; 16])),
            ("textures/pbr/walls/relief_p.dds", dx10(80, 4, 4, &[255, 0, 0, 0, 0, 0, 0, 0])),
            ("textures/pbr/walls/relief.toml", b"parallax_depth = 0.05\n".to_vec()),
            ("textures/walls/flat.dds", dxt1.clone()),
            ("textures/pbr/walls/flat_p.dds", legacy(b"BC4U", 4, 4, &[0; 8])),
            ("textures/pbr/walls/flat.toml", b"parallax_depth = 0\n".to_vec()),
            ("textures/walls/brokenheight.dds", dxt1.clone()),
            ("textures/pbr/walls/brokenheight_orm.dds", dx10(98, 4, 4, &[0; 16])),
            ("textures/pbr/walls/brokenheight_p.dds", b"not a dds".to_vec()),
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
    let authored = |s: Option<PbrSet>| match s {
        Some(PbrSet::Authored(a)) => Some(a),
        _ => None,
    };
    let set = authored(source.pbr_set(&key)).expect("authored set");
    assert_eq!(set.stem, r"textures\pbr\walls\brick");
    assert_eq!(set.scalars, Scalars { parallax_depth: None, roughness_scale: Some(0.5), reflectance: Some(0.25) });
    assert!(set.height.is_none(), "brick has no _p");
    let normal = set.normal.clone().expect("_n");
    let none = authored(source.pbr_set(&source.resolve("walls/nonormal.dds"))).expect("set without _n");
    assert!(none.normal.is_none());
    assert_eq!(none.scalars, Scalars::default());
    // Height only: the `_p` alone, at the default depth.
    let stone = source.pbr_set(&source.resolve("walls/stone.dds")).expect("height-only set");
    assert!(stone.authored().is_none(), "height only is no authored set");
    let stone_height = stone.height().cloned().expect("its height map");
    assert_eq!(
        (stone.stem(), stone_height.depth_scale, stone_height.method, stone_height.max_layers),
        (r"textures\pbr\walls\stone", fnv_pbr::DEFAULT_PARALLAX_DEPTH, PARALLAX_METHOD, MAX_PARALLAX_LAYERS)
    );
    // An authored set with `_p` and a depth in its scalar file.
    let relief = authored(source.pbr_set(&source.resolve("walls/relief.dds"))).expect("authored set with _p");
    let relief_height = relief.height.clone().expect("_p");
    assert_eq!(relief_height.depth_scale, 0.05);
    // A depth of 0: no height map, so a height-only set is no set at all and its `_p` is not read.
    assert!(source.pbr_set(&source.resolve("walls/flat.dds")).is_none(), "depth 0: nothing to draw");
    // A `_p` that does not load: an authored set keeps the rest.
    let broken = authored(source.pbr_set(&source.resolve("walls/brokenheight.dds"))).expect("the _orm still applies");
    assert!(broken.height.is_none());
    assert!(source.pbr_set(&source.resolve("walls/plain.dds")).is_none());
    assert!(source.pbr_set(&source.resolve("walls/bad.dds")).is_none(), "an _orm that does not load leaves the shape legacy");
    let twin = source.pbr_set(&source.resolve("weapons/1stpersongun.dds")).expect("the twin's own height-only set");
    assert_eq!((twin.stem(), twin.authored().is_none()), (r"textures\pbr\weapons\1stpersongun", true), "a height-only twin wins the lookup: no authored set, not the plain one's");
    assert_eq!(source.pbr_set(&source.resolve("weapons/gun.dds")).map(|s| s.stem().to_string()).as_deref(), Some(r"textures\pbr\weapons\gun"));
    // The console folder: the member the index names, not the PC file `get` would choose.
    let xbox_key = source.resolve(r"walls\xbox\tile.dds");
    assert_eq!(xbox_key, r"textures\walls\xbox\tile.dds", "no PC diffuse: the key stays");
    let xbox = authored(source.pbr_set(&xbox_key)).expect("xbox set");
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
    assert!(warned.contains(r"textures\pbr\walls\brokenheight_p.dds"), "{warned:?}");
    assert!(!cache.has_or_ahead(r"textures\pbr\walls\flat_p.dds"), "a depth of 0 loads no _p");
    // `_p` in the linear role, as unsigned BC4.
    for h in [stone_height, relief_height] {
        assert_eq!(images.get(h.depth_map.as_ref().unwrap()).unwrap().texture_descriptor.format, TextureFormat::Bc4RUnorm);
    }
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
    let cache = ScalarCache::default();
    let reads = AtomicU64::new(0);
    let read = || {
        reads.fetch_add(1, Ordering::Relaxed);
        Some(b"reflectance = 0.1\nbogus = 1\n".to_vec())
    };
    let a = scalars_cached(&cache, "textures/pbr/a.toml", read);
    let b = scalars_cached(&cache, "textures/pbr/a.toml", || panic!("read again"));
    assert_eq!((a, a.reflectance), (b, Some(0.1)));
    assert_eq!(reads.load(Ordering::Relaxed), 1);
    assert_eq!(scalars_cached(&cache, "textures/pbr/gone.toml", || None), Scalars::default());
}

/// Workers asking for the same file at once: one reads (and warns), the rest wait for it.
#[test]
fn simultaneous_first_reads_of_a_scalar_file_read_it_once() {
    let cache = ScalarCache::default();
    let reads = AtomicU64::new(0);
    let start = std::sync::Barrier::new(8);
    let got: Vec<Scalars> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    start.wait();
                    scalars_cached(&cache, "textures/pbr/walls/brick.toml", || {
                        reads.fetch_add(1, Ordering::Relaxed);
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        Some(b"roughness_scale = 0.5\nunknown = 2\n".to_vec())
                    })
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().unwrap()).collect()
    });
    assert_eq!(reads.load(Ordering::Relaxed), 1, "one read and one set of warnings");
    assert!(got.iter().all(|s| s.roughness_scale == Some(0.5)));
}

/// The scalar files belong to the mount: another VFS mounted in place reads them afresh.
#[test]
fn scalars_are_read_per_mount() {
    let pack = Pack::new();
    let path = r"textures\pbr\walls\brick.toml";
    assert_eq!(pack.data.pbr_scalars(path).roughness_scale, Some(0.5));
    let other = Pack::new();
    let mut moved = pack.data.clone();
    moved.set_vfs(other.data.vfs.clone());
    std::fs::write(other.root.join("textures/pbr/walls/brick.toml"), b"roughness_scale = 0.25\n").unwrap();
    assert_eq!(moved.pbr_scalars(path).roughness_scale, Some(0.25), "not the old mount's value");
    assert_eq!(pack.data.pbr_scalars(path).roughness_scale, Some(0.5), "the old mount keeps its own");
}

#[test]
fn a_height_only_set_is_no_authored_set_even_as_a_first_person_twin() {
    let index = PbrIndex::build([r"textures\pbr\a\stone_p.dds", r"textures\pbr\a\1stpersongun_p.dds", r"textures\pbr\a\gun_orm.dds", r"textures\pbr\a\knife_orm.dds", r"textures\pbr\a\1stpersonknife_n.dds"]);
    assert!(authored_found(&index, r"textures\a\stone.dds").is_none());
    assert!(authored_found(&index, r"textures\a\1stpersongun.dds").is_none(), "the height-only twin wins; the plain set is not used behind it");
    assert!(authored_found(&index, r"textures\a\gun.dds").is_some());
    // An incomplete twin is no active set, so the plain set serves first person.
    let knife = authored_found(&index, r"textures\a\1stpersonknife.dds").unwrap();
    assert_eq!((knife.stem.as_str(), knife.first_person_fallback), (r"textures\pbr\a\knife", true));
}

/// The swap rule (`retexture`): authored → another set → no set, always from the legacy look,
/// for a shape whose drawn mesh has tangents and for one without (its later `_n` is not
/// applied); a material that takes no set is left alone.
#[test]
fn retexturing_goes_from_set_to_set_to_legacy_from_the_legacy_look() {
    use crate::nif_spawn::{shape_material, Props};
    use crate::texture::bc5_tests::triangle;
    let pack = Pack::new();
    let mut cache = TextureCache::default();
    let mut images = Assets::<Image>::default();
    let props = |t: &str| {
        let mut p = Props::default();
        p.textures = vec![t.into()];
        p
    };
    // Drawn with brick (its `_n` gives the mesh tangents), then with plain (no set, no tangents).
    let (with_n, _) = shape_material(&props(r"walls\brick.dds"), &mut triangle(), &mut cache, &pack.data, &mut images, None);
    let (bare, _) = shape_material(&props(r"walls\plain.dds"), &mut triangle(), &mut cache, &pack.data, &mut images, None);
    assert!(is_authored(&with_n) && with_n.base.normal_map_texture.is_some());
    assert!(!is_authored(&bare) && !bare.extension.legacy.as_ref().unwrap().tangents);
    let brick_n = with_n.base.normal_map_texture.clone();
    for (start, tangents) in [(with_n, true), (bare, false)] {
        let legacy = start.extension.legacy.clone().unwrap();
        let mut m = start.clone();
        // Card A: brick's set.
        assert_eq!(retexture(&mut m, r"walls\brick.dds", &mut cache, &pack.data, &mut images, None), Retextured::Authored);
        assert_eq!(m.base.normal_map_texture.is_some(), tangents, "brick's _n only where the drawn mesh has tangents");
        assert_eq!((m.base.reflectance, m.base.perceptual_roughness), (0.25, 0.5));
        // Card B, from the copy card A wore: its own set, nothing of brick's.
        let mut b = m.clone();
        assert_eq!(retexture(&mut b, r"walls\nonormal.dds", &mut cache, &pack.data, &mut images, None), Retextured::Authored);
        assert_eq!(b.base.normal_map_texture, legacy.normal, "no _n: the legacy normal state, never brick's");
        assert_ne!(b.base.metallic_roughness_texture, m.base.metallic_roughness_texture);
        assert_eq!((b.base.reflectance, b.base.perceptual_roughness), (0.5, 1.0), "the scalars' defaults");
        // No set: the legacy look back, mark and maps gone.
        assert_eq!(retexture(&mut b, r"walls\plain.dds", &mut cache, &pack.data, &mut images, None), Retextured::Legacy);
        assert!(!is_authored(&b) && b.base.metallic_roughness_texture.is_none() && b.base.occlusion_texture.is_none());
        assert_eq!((b.base.normal_map_texture.clone(), b.base.specular_texture.clone(), b.base.reflectance, b.base.perceptual_roughness, b.base.metallic), (legacy.normal.clone(), legacy.specular.clone(), legacy.reflectance, legacy.roughness, legacy.metallic));
        assert_eq!(b.extension.legacy, Some(legacy), "the legacy look itself never changes");
    }
    assert!(brick_n.is_some());
    // A material that takes no set (no legacy look): untouched.
    let mut fx = crate::nif_material::nif_material(StandardMaterial { unlit: true, ..default() });
    assert_eq!(retexture(&mut fx, r"walls\brick.dds", &mut cache, &pack.data, &mut images, None), Retextured::Untouched);
    assert!(fx.base.metallic_roughness_texture.is_none());
    // Made legacy for good (a FaceGen target, the hair model): no later set either.
    let (mut skin, _) = shape_material(&props(r"walls\brick.dds"), &mut triangle(), &mut cache, &pack.data, &mut images, None);
    assert!(make_legacy(&mut skin) && !is_authored(&skin));
    assert_eq!(retexture(&mut skin, r"walls\brick.dds", &mut cache, &pack.data, &mut images, None), Retextured::Untouched);
}

#[test]
fn channel_views_parse_and_name_their_defs() {
    assert_eq!(PbrView::parse(""), Some(PbrView::Off));
    assert_eq!(PbrView::parse(" Roughness "), Some(PbrView::Roughness));
    assert_eq!(PbrView::parse("bogus"), None);
    for v in PbrView::ALL {
        assert_eq!(PbrView::parse(v.name()), Some(v));
        let defs = v.shader_defs();
        assert_eq!(defs[0], "NVF_PBR_VIEW");
        assert!(defs[1].ends_with(&v.name().to_ascii_uppercase()));
    }
    assert!(PbrView::Off.shader_defs().is_empty());
    // The tests run without the variables: no view, no metallic switch, metallic held at 0.
    if std::env::var_os(VIEW_ENV).is_none() && std::env::var_os(METALLIC_ENV).is_none() {
        assert!(!metallic_debug());
        assert_eq!((view(), authored_metallic()), (PbrView::Off, 0.0));
    }
}

/// Landscape LOD models take no set, through the model loader and the direct spawn alike.
#[test]
fn landscape_lod_models_are_recognised_whatever_the_spelling() {
    assert!(crate::model::is_lod_model("Meshes/Landscape/LOD/WastelandNV/WastelandNV.level4.x0.y0.nif"));
    assert!(crate::model::is_lod_model("meshes/landscape/lod/wastelandnv/blocks/wastelandnv.level4.x0.y0.nif"));
    assert!(!crate::model::is_lod_model("meshes/architecture/goodsprings/house01.nif"));
}

/// The exit summary's counts read back in its line (a run's own counters are global; this
/// one is fresh).
#[test]
fn the_summary_line_names_every_count() {
    let s = Stats::default();
    for (c, n) in [(&s.built, 3), (&s.not_eligible, 5), (&s.excluded_texture, 1), (&s.orm_failed, 2), (&s.normal_without_tangents, 4), (&s.made_legacy, 6), (&s.swaps_authored, 7), (&s.swaps_legacy, 8)] {
        for _ in 0..n {
            Stats::add(c);
        }
    }
    s.stems.lock().unwrap().extend(["a".to_string(), "b".to_string()]);
    assert_eq!(
        s.line(),
        "3 material builds applied a set (2 distinct set stems at build); builds without one: 5 not eligible, 1 on excluded textures, 2 whose _orm did not load; 4 applied without their _n (no tangents); 6 role copies made legacy; 15 retexture calls (7 to a set, 8 to the legacy look)"
    );
}

/// The role pass's gate: off, or a mount without authored sets, means no shape can take one.
#[test]
fn a_mount_has_sets_only_with_the_setting_on_and_an_authored_set() {
    let pack = Pack::new();
    assert!(!mount_has_sets_when(&pack.data, false));
    assert!(!pack.data.pbr_index_built(), "off asks nothing of the index");
    assert!(mount_has_sets_when(&pack.data, true));
    let empty = std::env::temp_dir().join(format!("nvf-authored-empty-{}", std::process::id()));
    std::fs::create_dir_all(empty.join("textures")).unwrap();
    let data = GameData::new(Arc::new(fnv_vfs::Vfs::open(&empty, &[]).unwrap()), Arc::new(fnv_nif::Schema::default()));
    assert!(!mount_has_sets_when(&data, true), "no authored set in the mount");
    let _ = std::fs::remove_dir_all(&empty);
}
