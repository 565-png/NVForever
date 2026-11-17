use super::*;
use crate::texture::{
    bc5_tests::OwnedData, sampler, worker_texture, TextureSnapshot, TextureUsage,
};
use bevy::image::ImageAddressMode;
use std::sync::Mutex;
static LOCK: Mutex<()> = Mutex::new(());
struct Restore(u16);
impl Drop for Restore {
    fn drop(&mut self) {
        set_anisotropy(self.0);
    }
}
fn level(image: &Image) -> u16 {
    let ImageSampler::Descriptor(s) = &image.sampler else {
        panic!("DDS descriptor")
    };
    s.anisotropy_clamp
}
fn saved() -> Restore {
    let ImageSampler::Descriptor(s) = sampler() else {
        unreachable!()
    };
    Restore(s.anisotropy_clamp)
}

#[test]
fn command_changes_current_role_images_without_replacing_handles_or_other_fields() {
    let _lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = saved();
    set_anisotropy(8);
    let owned = OwnedData::new();
    let mut cache = TextureCache::default();
    let mut images = Assets::default();
    let color = cache
        .get("old.dds", true, &owned.data, &mut images, None)
        .unwrap();
    let linear = cache
        .get("old.dds", false, &owned.data, &mut images, None)
        .unwrap();
    let normal = cache
        .normal("signed.dds", &owned.data, &mut images, None)
        .unwrap();
    let clamped = cache
        .get_clamped("old.dds", true, &owned.data, &mut images, None)
        .unwrap();
    let foreign = images.add(images.get(&color).unwrap().clone());
    let before: Vec<_> = [&color, &linear, &normal.0, &clamped]
        .into_iter()
        .map(|h| images.get(h).unwrap().clone())
        .collect();
    assert_eq!(cache.apply_anisotropy(&mut images, 16), 4);
    for (h, prior) in [&color, &linear, &normal.0, &clamped]
        .into_iter()
        .zip(before)
    {
        let now = images.get(h).unwrap();
        assert_eq!(level(now), 16);
        assert_eq!(now.data, prior.data);
        assert_eq!(
            now.texture_descriptor.format,
            prior.texture_descriptor.format
        );
        assert_eq!(now.texture_descriptor.size, prior.texture_descriptor.size);
        assert_eq!(
            now.texture_descriptor.mip_level_count,
            prior.texture_descriptor.mip_level_count
        );
        let (ImageSampler::Descriptor(a), ImageSampler::Descriptor(b)) =
            (&now.sampler, &prior.sampler)
        else {
            unreachable!()
        };
        assert_eq!(
            (
                a.address_mode_u,
                a.address_mode_v,
                a.min_filter,
                a.mag_filter,
                a.mipmap_filter
            ),
            (
                b.address_mode_u,
                b.address_mode_v,
                b.min_filter,
                b.mag_filter,
                b.mipmap_filter
            )
        );
    }
    assert_eq!(level(images.get(&foreign).unwrap()), 8);
    assert_eq!(
        cache
            .normal("signed.dds", &owned.data, &mut images, None)
            .unwrap(),
        normal
    );
    assert_eq!(
        cache.get("old.dds", true, &owned.data, &mut images, None),
        Some(color)
    );
    let ImageSampler::Descriptor(s) = &images.get(&clamped).unwrap().sampler else {
        unreachable!()
    };
    assert_eq!(s.address_mode_u, ImageAddressMode::ClampToEdge);
}

#[test]
fn cold_and_both_pending_worker_publications_use_current_filtering() {
    let _lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = saved();
    set_anisotropy(8);
    let owned = OwnedData::new();
    let mut cache = TextureCache::default();
    let mut images = Assets::default();
    let provider = images.get_handle_provider();
    let empty = TextureSnapshot::new();
    let normal = worker_texture(
        "signed.dds",
        TextureUsage::Normal,
        &owned.data,
        &empty,
        &cache.workers,
        &provider,
        true,
    )
    .unwrap();
    let linear = worker_texture(
        "unsigned.dds",
        TextureUsage::Linear,
        &owned.data,
        &empty,
        &cache.workers,
        &provider,
        true,
    )
    .unwrap();
    cache.apply_anisotropy(&mut images, 16);
    assert_eq!(
        cache
            .normal("signed.dds", &owned.data, &mut images, None)
            .unwrap(),
        (normal.0.clone(), normal.2)
    );
    assert_eq!(level(images.get(&normal.0).unwrap()), 16);
    assert_eq!(cache.take_workers(&mut images, 0.0), 1);
    assert_eq!(level(images.get(&linear.0).unwrap()), 16);
    let cold = cache
        .get("old.dds", true, &owned.data, &mut images, None)
        .unwrap();
    assert_eq!(level(images.get(&cold).unwrap()), 16);
    assert_ne!(cold, normal.0);
    assert_ne!(normal.0, linear.0);
}

#[test]
fn request_is_not_a_held_overlay_and_foreign_sampler_fields_are_not_rewritten() {
    let _lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = saved();
    let owned = OwnedData::new();
    let mut cache = TextureCache::default();
    let mut images = Assets::default();
    let h = cache
        .get("old.dds", true, &owned.data, &mut images, None)
        .unwrap();
    cache.apply_anisotropy(&mut images, 16);
    {
        let mut image = images.get_mut(&h).unwrap();
        let ImageSampler::Descriptor(s) = &mut image.sampler else {
            unreachable!()
        };
        s.anisotropy_clamp = 4;
        s.address_mode_u = ImageAddressMode::ClampToEdge;
    }
    // A cache read is not a new command and must not restore16 over this equal-handle write.
    assert_eq!(
        cache.get("old.dds", true, &owned.data, &mut images, None),
        Some(h.clone())
    );
    assert_eq!(level(images.get(&h).unwrap()), 4);
    {
        let mut image = images.get_mut(&h).unwrap();
        let ImageSampler::Descriptor(s) = &mut image.sampler else {
            unreachable!()
        };
        s.min_filter = ImageFilterMode::Nearest;
        s.anisotropy_clamp = 1;
    }
    assert_eq!(cache.apply_anisotropy(&mut images, 16), 0);
    let ImageSampler::Descriptor(s) = &images.get(&h).unwrap().sampler else {
        unreachable!()
    };
    assert_eq!(
        (s.min_filter, s.address_mode_u, s.anisotropy_clamp),
        (ImageFilterMode::Nearest, ImageAddressMode::ClampToEdge, 1)
    );
    {
        let mut image = images.get_mut(&h).unwrap();
        image.sampler = ImageSampler::Default;
    }
    assert_eq!(cache.apply_anisotropy(&mut images, 8), 0);
    assert!(matches!(
        images.get(&h).unwrap().sampler,
        ImageSampler::Default
    ));
    images.remove(h.id());
    assert_eq!(
        cache.apply_anisotropy(&mut images, 8),
        0,
        "removed assets are not recreated by stale cache membership"
    );
}

#[test]
fn a_real_worker_in_flight_publishes_the_last_request_without_a_duplicate_handle() {
    let _lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _restore = saved();
    set_anisotropy(8);
    let owned = OwnedData::new();
    let mut cache = TextureCache::default();
    let mut images = Assets::default();
    let workers = cache.workers.clone();
    let provider = images.get_handle_provider();
    let key = ("textures\\old.dds".to_string(), TextureUsage::Color);
    let bytes = owned.data.read("textures\\old.dds").unwrap();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let loaded = std::thread::scope(|scope| {
        let task = scope.spawn(move || {
            workers
                .load_once(key, &provider, |_| {
                    let image = crate::texture::dds_image(&bytes, true, true).unwrap();
                    assert_eq!(level(&image), 8);
                    ready_tx.send(()).unwrap();
                    release_rx
                        .recv_timeout(std::time::Duration::from_secs(3))
                        .unwrap();
                    Ok(image.into())
                })
                .unwrap()
        });
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        cache.apply_anisotropy(&mut images, 16);
        release_tx.send(()).unwrap();
        task.join().unwrap()
    });
    assert_eq!(cache.take_workers(&mut images, 0.0), 1);
    assert_eq!(level(images.get(&loaded.0).unwrap()), 16);
    assert_eq!(
        cache.get("old.dds", true, &owned.data, &mut images, None),
        Some(loaded.0)
    );
    assert_eq!(cache.take_workers(&mut images, 1.0), 0);
}
