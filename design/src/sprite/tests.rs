use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use super::*;
use crate::tokens::color;
use crate::tokens::motion::SPRITE_FRAME_MIN_MS;

fn loaded() -> &'static SpriteSheet {
    sheet().expect("embedded sprite sheet must decode")
}

/// Il manifest riletto come JSON generico: indipendente dal parser del crate.
fn manifest_values() -> Vec<serde_json::Value> {
    let json: serde_json::Value = serde_json::from_slice(MANIFEST_JSON).unwrap();
    json["sprites"].as_array().unwrap().clone()
}

fn colors_of(rgba: &[u8]) -> BTreeSet<[u8; 4]> {
    rgba.as_chunks::<4>().0.iter().copied().collect()
}

#[test]
fn loads_all_72_sprites_with_the_frame_counts_of_sprites_json() {
    let sheet = loaded();
    let expected = manifest_values();
    assert_eq!(expected.len(), 72);
    assert_eq!(sheet.len(), 72);
    for value in &expected {
        let name = value["name"].as_str().unwrap();
        let sprite = sheet.get(name).unwrap_or_else(|| panic!("missing sprite {name}"));
        let frames = value["frames"].as_u64().unwrap() as usize;
        assert_eq!(sprite.frame_count(), frames, "{name}");
        let size = value["frame_size"].as_array().unwrap();
        let (w, h) = (size[0].as_u64().unwrap() as u32, size[1].as_u64().unwrap() as u32);
        assert_eq!(sprite.frame_size(), (w, h), "{name}");
        let durations: Vec<u64> =
            value["durations_ms"].as_array().unwrap().iter().map(|d| d.as_u64().unwrap()).collect();
        let loaded: Vec<u64> = sprite.frames().iter().map(|f| u64::from(f.duration_ms)).collect();
        assert_eq!(loaded, durations, "{name}");
        for frame in sprite.frames() {
            assert_eq!((frame.width, frame.height), (w, h));
            assert_eq!(frame.rgba.len(), (w * h * 4) as usize);
        }
    }
}

#[test]
fn name_constants_cover_exactly_the_manifest() {
    let from_json: Vec<String> =
        manifest_values().iter().map(|v| v["name"].as_str().unwrap().to_owned()).collect();
    let from_consts: Vec<String> = ALL_NAMES.iter().map(|n| (*n).to_owned()).collect();
    assert_eq!(
        from_consts, from_json,
        "names.rs and sprites.json must list the same names in the same order"
    );
    let unique: HashSet<_> = ALL_NAMES.iter().collect();
    assert_eq!(unique.len(), ALL_NAMES.len());
    assert_eq!(EMBEDDED_PNGS.len(), ALL_NAMES.len());
    assert!(loaded().get(ICON_HEART_FILL).is_some());
    assert!(loaded().get(NASTRO_LISTEN).is_some());
    assert!(loaded().get("icon_does_not_exist").is_none());
}

#[test]
fn sprite_sheet_is_decoded_once() {
    let a = sheet().unwrap() as *const SpriteSheet;
    let b = sheet().unwrap() as *const SpriteSheet;
    assert_eq!(a, b);
}

#[test]
fn decoded_memory_cost_is_pinned() {
    // Documented in `mod.rs`: update the module comment when it changes.
    assert_eq!(loaded().decoded_bytes(), 159_744);
}

#[test]
fn groups_kinds_and_animation_rules() {
    let sheet = loaded();
    let masks = sheet.iter().filter(|s| s.kind() == SpriteKind::Mask).count();
    assert_eq!(masks, 64); // 59 icons + eq, eq_rest, mark, mark_play, liked_cover
    for sprite in sheet.iter() {
        if sprite.group() == "icons" {
            assert_eq!(sprite.kind(), SpriteKind::Mask, "{}", sprite.name());
        }
        if sprite.frame_count() > 1 {
            for frame in sprite.frames() {
                assert!(frame.duration_ms >= SPRITE_FRAME_MIN_MS, "{}", sprite.name());
            }
        }
    }
    for name in [ICON_PLAY, ALBUM_PLACEHOLDER, APP_ICON] {
        assert_eq!(sheet.get(name).unwrap().frame_count(), 1);
    }
    assert_eq!(sheet.get(NASTRO_IDLE).unwrap().frame_count(), 4);
    assert_eq!(sheet.get(LOADER).unwrap().frame_count(), 8);
}

#[test]
fn masks_use_one_color_plus_transparency() {
    for sprite in loaded().iter().filter(|s| s.kind() == SpriteKind::Mask) {
        for frame in sprite.frames() {
            let opaque: BTreeSet<_> = colors_of(&frame.rgba).into_iter().filter(|px| px[3] != 0).collect();
            assert!(opaque.len() <= 1, "{}: {opaque:?}", sprite.name());
            assert!(opaque.iter().all(|px| px[3] == 255));
        }
    }
}

#[test]
fn color_sprites_only_use_palette_tokens() {
    let allowed: BTreeSet<[u8; 3]> = color::PALETTE.iter().map(|(_, c)| [c.r(), c.g(), c.b()]).collect();
    for sprite in loaded().iter().filter(|s| s.kind() == SpriteKind::Color) {
        for frame in sprite.frames() {
            for px in colors_of(&frame.rgba) {
                assert!(px[3] == 0 || allowed.contains(&[px[0], px[1], px[2]]), "{}: {px:?}", sprite.name());
                assert!(px[3] == 0 || px[3] == 255, "{}: alfa parziale", sprite.name());
            }
        }
    }
}

#[test]
fn upscale_keeps_the_exact_color_set_and_makes_flat_blocks() {
    for sprite in loaded().iter() {
        for (index, frame) in sprite.frames().iter().enumerate() {
            for k in [1u32, 2, 3, 4, 5, 7, 8] {
                let up = upscale_nearest(frame, k);
                assert_eq!((up.width, up.height), (frame.width * k, frame.height * k));
                assert_eq!(up.rgba.len(), (up.width * up.height * 4) as usize);
                assert_eq!(colors_of(&up.rgba), colors_of(&frame.rgba), "{} #{index} x{k}", sprite.name());
                // Ogni pixel dell'uscita è il pixel sorgente (x / k, y / k): blocchi k x k piatti.
                let stride = (up.width * 4) as usize;
                for y in 0..up.height as usize {
                    for x in 0..up.width as usize {
                        let (sx, sy) = (x / k as usize, y / k as usize);
                        let s = (sy * frame.width as usize + sx) * 4;
                        let o = y * stride + x * 4;
                        assert_eq!(up.rgba[o..o + 4], frame.rgba[s..s + 4]);
                    }
                }
            }
        }
    }
}

#[test]
fn upscale_factor_edges() {
    let frame = &loaded().get(ICON_PLAY).unwrap().frames()[0];
    assert_eq!(upscale_nearest(frame, 0), upscale_nearest(frame, 1));
    assert_eq!(upscale_nearest(frame, 1).rgba, frame.rgba);
    let huge = upscale_nearest(frame, 1000);
    assert_eq!(huge.width, frame.width * MAX_FACTOR);
}

#[test]
fn tinting_replaces_only_opaque_pixels() {
    let sprite = loaded().get(ICON_HEART).unwrap();
    let frame = &sprite.frames()[0];
    let tint = color::LIME;
    let up = upscale_tinted(frame, 3, tint);
    let colors = colors_of(&up.rgba);
    assert!(colors.contains(&tint.bytes()));
    assert!(colors.iter().all(|px| *px == tint.bytes() || px[3] == 0));
    // La forma (maschera alfa) non cambia rispetto all'ingrandimento semplice.
    let plain = upscale_nearest(frame, 3);
    let alpha = |p: &Pixels| p.rgba.as_chunks::<4>().0.iter().map(|px| px[3] != 0).collect::<Vec<_>>();
    assert_eq!(alpha(&up), alpha(&plain));
    // Una tinta traslucida scala l'alfa.
    let half = upscale_tinted(frame, 1, color::SCRIM);
    assert!(half.rgba.as_chunks::<4>().0.iter().all(|px| px[3] == 0 || px[3] == 184));
}

#[test]
fn frame_index_follows_the_durations_in_a_loop() {
    let idle = loaded().get(NASTRO_IDLE).unwrap(); // 1000, 240, 1000, 120
    assert_eq!(idle.cycle_ms(), 2360);
    assert_eq!(idle.frame_index_at(0), 0);
    assert_eq!(idle.frame_index_at(999), 0);
    assert_eq!(idle.frame_index_at(1000), 1);
    assert_eq!(idle.frame_index_at(1240), 2);
    assert_eq!(idle.frame_index_at(2240), 3);
    assert_eq!(idle.frame_index_at(2360), 0);
    assert_eq!(idle.frame_index_at(2360 * 1000 + 1000), 1);
    let still = loaded().get(ICON_PLAY).unwrap();
    assert_eq!(still.frame_index_at(123_456), 0);
}

#[test]
fn cache_hits_share_the_allocation_and_count_bytes() {
    let sprite = loaded().get(NASTRO_LISTEN).unwrap();
    let mut cache = UpscaleCache::new();
    let a = cache.get_or_upscale(sprite, 1, 4, None).unwrap();
    let b = cache.get_or_upscale(sprite, 1, 4, None).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.bytes(), 32 * 4 * 32 * 4 * 4);
    cache.get_or_upscale(sprite, 2, 4, None).unwrap();
    cache.get_or_upscale(sprite, 1, 5, None).unwrap();
    assert_eq!(cache.len(), 3);
    assert!(cache.get_or_upscale(sprite, 99, 4, None).is_none());
    cache.clear();
    assert!(cache.is_empty());
    assert_eq!(cache.bytes(), 0);
}

#[test]
fn cache_key_includes_tint_only_for_masks() {
    let mask = loaded().get(ICON_PLAY).unwrap();
    let art = loaded().get(NASTRO_LISTEN).unwrap();
    let mut cache = UpscaleCache::new();
    let white = cache.get_or_upscale(mask, 0, 2, Some(color::WARM_WHITE)).unwrap();
    let lime = cache.get_or_upscale(mask, 0, 2, Some(color::LIME)).unwrap();
    assert_ne!(white.rgba, lime.rgba);
    assert_eq!(cache.len(), 2);
    let first = cache.get_or_upscale(art, 0, 2, Some(color::LIME)).unwrap();
    let second = cache.get_or_upscale(art, 0, 2, None).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(cache.len(), 3);
}

#[test]
fn cache_evicts_least_recently_used_by_entry_count() {
    let sprite = loaded().get(ICON_PLAY).unwrap();
    let mut cache = UpscaleCache::with_capacity(2, usize::MAX);
    let first = cache.get_or_upscale(sprite, 0, 1, None).unwrap();
    cache.get_or_upscale(sprite, 0, 2, None).unwrap();
    cache.get_or_upscale(sprite, 0, 1, None).unwrap(); // rinfresca il fattore 1
    cache.get_or_upscale(sprite, 0, 3, None).unwrap(); // scarta il fattore 2
    assert_eq!(cache.len(), 2);
    assert!(Arc::ptr_eq(&first, &cache.get_or_upscale(sprite, 0, 1, None).unwrap()));
    let before = cache.len();
    cache.get_or_upscale(sprite, 0, 2, None).unwrap(); // ricalcolato: scarta il fattore 3
    assert_eq!(cache.len(), before);
}

#[test]
fn cache_respects_the_byte_budget() {
    let sprite = loaded().get(ICON_PLAY).unwrap(); // 12 x 12 AP, 4 byte a pixel
    let at = |k: usize| 12 * k * 12 * k * 4;
    let mut cache = UpscaleCache::with_capacity(100, at(4) + 1000);
    cache.get_or_upscale(sprite, 0, 2, None).unwrap();
    cache.get_or_upscale(sprite, 0, 3, None).unwrap();
    assert_eq!((cache.len(), cache.bytes()), (2, at(2) + at(3)));
    cache.get_or_upscale(sprite, 0, 4, None).unwrap(); // non ci sta: sloggia il più vecchio finché serve
    assert!(cache.bytes() <= cache.max_bytes());
    assert_eq!((cache.len(), cache.bytes()), (1, at(4)));
    // Una voce da sola oltre il budget si restituisce ma non si conserva.
    let mut tiny = UpscaleCache::with_capacity(8, at(1));
    let out = tiny.get_or_upscale(sprite, 0, 2, None).unwrap();
    assert_eq!(out.width, 24);
    assert!(tiny.is_empty());
    // Capacità zero: mai nulla in cache.
    let mut none = UpscaleCache::with_capacity(0, usize::MAX);
    assert!(none.get_or_upscale(sprite, 0, 2, None).is_some());
    assert!(none.is_empty());
}

#[test]
fn default_cache_is_bounded_under_the_declared_budget() {
    let cache = UpscaleCache::new();
    assert_eq!(cache.max_entries(), 128);
    assert_eq!(cache.max_bytes(), 8 * 1024 * 1024);
    // Il caso peggiore documentato: mascotte a ap_device 8 = 256 KiB, quindi 32 voci riempiono il budget.
    let mut cache = UpscaleCache::new();
    let sheet = loaded();
    for sprite in sheet.iter().filter(|s| s.frame_size() == (32, 32)) {
        for frame in 0..sprite.frame_count() {
            for tint in [None, Some(color::LIME)] {
                cache.get_or_upscale(sprite, frame, 8, tint).unwrap();
                assert!(cache.bytes() <= cache.max_bytes() && cache.len() <= cache.max_entries());
            }
        }
    }
}

#[test]
fn broken_inputs_are_reported_not_panicked() {
    let png = |name: &str| EMBEDDED_PNGS.iter().find(|(n, _)| *n == name).map(|(_, b)| *b).unwrap();
    let entry = |name: &str, frames: u32, w: u32, durations: &str| {
        format!(
            r#"{{"ap_dp":2,"sprites":[{{"name":"{name}","group":"icons","file":"x","frame_size":[{w},12],
            "frames":{frames},"durations_ms":{durations},"palette":["transparent","tint"]}}]}}"#
        )
    };
    let load =
        |json: String, pngs: &[(&str, &[u8])]| SpriteSheet::from_parts(json.as_bytes(), pngs).unwrap_err();
    assert!(matches!(load("not json".into(), &[]), SpriteError::Manifest(_)));
    assert!(matches!(
        load(entry("icon_play", 1, 12, "[0]").replace("\"ap_dp\":2", "\"ap_dp\":3"), &[]),
        SpriteError::Manifest(_)
    ));
    assert!(matches!(load(entry("icon_play", 1, 12, "[0]"), &[]), SpriteError::MissingPng(_)));
    let good = [("icon_play", png("icon_play"))];
    assert!(matches!(load(entry("icon_play", 1, 13, "[0]"), &good), SpriteError::Shape { .. }));
    assert!(matches!(load(entry("icon_play", 2, 12, "[0]"), &good), SpriteError::Shape { .. }));
    assert!(matches!(load(entry("icon_play", 0, 12, "[]"), &good), SpriteError::Shape { .. }));
    let corrupt = [("icon_play", &png("icon_play")[..20])];
    assert!(matches!(load(entry("icon_play", 1, 12, "[0]"), &corrupt), SpriteError::Decode { .. }));
    let ok = SpriteSheet::from_parts(entry("icon_play", 1, 12, "[0]").as_bytes(), &good).unwrap();
    assert_eq!(ok.len(), 1);
    assert!(!SpriteError::MissingPng("x".into()).to_string().is_empty());
}
