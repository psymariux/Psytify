// These tests deliberately assert properties of constants: the «assertions_on_constants» lint does not apply.
#![allow(clippy::assertions_on_constants)]

//! Consistency checks of the tokens with each other and with the document `PSYTIFY_DESIGN_TOKENS.md`.

use std::collections::HashSet;

use psytify_design::tokens::color::{self, *};
use psytify_design::tokens::{interaction, motion, shape, size, space, type_, z};

#[test]
fn palette_has_twelve_distinct_opaque_colors_with_the_documented_bytes() {
    assert_eq!(PALETTE.len(), 12);
    let values: HashSet<_> = PALETTE.iter().map(|(_, c)| c.bytes()).collect();
    let names: HashSet<_> = PALETTE.iter().map(|(n, _)| *n).collect();
    assert_eq!(values.len(), 12, "due grigi hanno lo stesso valore");
    assert_eq!(names.len(), 12);
    assert!(PALETTE.iter().all(|(_, c)| c.a() == 255));
    assert_eq!(DEEP_BLACK.bytes(), [0, 0, 0, 255]);
    assert_eq!(NEAR_BLACK.bytes(), [14, 13, 12, 255]);
    assert_eq!(GRAPHITE.bytes(), [26, 25, 23, 255]);
    assert_eq!(CHARCOAL.bytes(), [42, 40, 38, 255]);
    assert_eq!(IRON.bytes(), [61, 58, 55, 255]);
    assert_eq!(STONE.bytes(), [120, 116, 109, 255]);
    assert_eq!(MEDIUM_GREY.bytes(), [154, 149, 141, 255]);
    assert_eq!(SOFT_GREY.bytes(), [189, 184, 175, 255]);
    assert_eq!(WARM_WHITE.bytes(), [236, 232, 224, 255]);
    assert_eq!(PURE_WHITE.bytes(), [255, 255, 255, 255]);
    assert_eq!(LIME.bytes(), [198, 242, 78, 255]);
    assert_eq!(ERROR.bytes(), [255, 107, 94, 255]);
}

#[test]
fn hex_values_match_the_doc_and_the_sprite_manifest() {
    let doc = [
        ("DEEP_BLACK", 0x000000),
        ("NEAR_BLACK", 0x0e0d0c),
        ("GRAPHITE", 0x1a1917),
        ("CHARCOAL", 0x2a2826),
        ("IRON", 0x3d3a37),
        ("STONE", 0x78746d),
        ("MEDIUM_GREY", 0x9a958d),
        ("SOFT_GREY", 0xbdb8af),
        ("WARM_WHITE", 0xece8e0),
        ("PURE_WHITE", 0xffffff),
        ("LIME", 0xc6f24e),
        ("ERROR", 0xff6b5e),
    ];
    for ((name, color), (doc_name, hex)) in PALETTE.iter().zip(doc) {
        assert_eq!(*name, doc_name);
        assert_eq!(color.hex(), hex, "{name}");
    }
    // `design/assets/pixel/manifest.toml` repeats the same twelve values: they must stay equal.
    let manifest = include_str!("../assets/pixel/manifest.toml");
    for (name, color) in PALETTE {
        let line = format!("{} = \"#{:06x}\"", name.to_lowercase(), color.hex());
        let squeezed: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            manifest.lines().any(|l| l.split_whitespace().collect::<Vec<_>>().join(" ") == squeezed),
            "manifest.toml non ha `{squeezed}`"
        );
    }
}

#[test]
fn the_palette_is_slightly_warm() {
    // All the greys: R - B between 2 and 14 (except pure black and white, and the two accent colours).
    for (name, c) in &PALETTE[1..9] {
        let warmth = i32::from(c.r()) - i32::from(c.b());
        assert!((2..=14).contains(&warmth), "{name}: {warmth}");
    }
}

#[test]
fn scrim_is_the_only_translucent_color() {
    assert_eq!(SCRIM.bytes(), [0, 0, 0, 184]);
    assert_eq!(SCRIM, DEEP_BLACK.with_alpha(184));
    assert_eq!(SCRIM.with_alpha(255), DEEP_BLACK);
    let aliases = [
        SURFACE_CANVAS,
        SURFACE_CHROME,
        SURFACE_RAISED,
        SURFACE_SELECTED,
        SURFACE_WELL,
        LINE_SUBTLE,
        LINE_STRONG,
        CONTROL_BORDER,
        CONTROL_BORDER_HOVER,
        FOCUS_RING,
        LEDGE_PRIMARY,
        LEDGE_SECONDARY,
        SHADOW_FLOAT,
        TEXT_PRIMARY,
        TEXT_SECONDARY,
        TEXT_TERTIARY,
        TEXT_DISABLED,
        TEXT_ON_LIGHT,
        TEXT_ERROR,
        ICON_IDLE,
        ICON_ACTIVE,
        ICON_DISABLED,
        STATE_ON,
        CARET,
    ];
    assert!(aliases.iter().all(|c| c.a() == 255));
}

#[test]
fn aliases_point_where_the_doc_says() {
    assert_eq!(SURFACE_CANVAS, DEEP_BLACK);
    assert_eq!(SURFACE_CHROME, NEAR_BLACK);
    assert_eq!(SURFACE_RAISED, GRAPHITE);
    assert_eq!(SURFACE_SELECTED, CHARCOAL);
    assert_eq!(SURFACE_WELL, DEEP_BLACK);
    assert_eq!((LINE_SUBTLE, LINE_STRONG), (CHARCOAL, IRON));
    assert_eq!((CONTROL_BORDER, CONTROL_BORDER_HOVER), (STONE, SOFT_GREY));
    assert_eq!((FOCUS_RING, CARET), (PURE_WHITE, PURE_WHITE));
    assert_eq!((LEDGE_PRIMARY, LEDGE_SECONDARY), (STONE, CHARCOAL));
    assert_eq!(SHADOW_FLOAT, DEEP_BLACK);
    assert_eq!(
        [TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY, TEXT_DISABLED, TEXT_ON_LIGHT, TEXT_ERROR],
        [WARM_WHITE, SOFT_GREY, MEDIUM_GREY, STONE, DEEP_BLACK, ERROR]
    );
    assert_eq!([ICON_IDLE, ICON_ACTIVE, ICON_DISABLED, STATE_ON], [SOFT_GREY, WARM_WHITE, STONE, LIME]);
    assert_eq!(color::SURFACES, [DEEP_BLACK, NEAR_BLACK, GRAPHITE, CHARCOAL]);
}

#[test]
fn text_tiers_and_interactive_roles_are_told_apart() {
    // Where the document says that two roles differ, the values differ.
    let tiers = [TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY, TEXT_DISABLED];
    assert_eq!(tiers.iter().collect::<HashSet<_>>().len(), 4);
    assert_ne!(CONTROL_BORDER, CONTROL_BORDER_HOVER);
    assert_ne!(LEDGE_PRIMARY, LEDGE_SECONDARY);
    assert_ne!(LINE_SUBTLE, LINE_STRONG);
    assert_ne!(STATE_ON, TEXT_ERROR);
    assert_ne!(SURFACE_RAISED, SURFACE_SELECTED);
    assert_ne!(SURFACE_CHROME, SURFACE_CANVAS);
}

#[test]
fn spacing_is_a_strictly_increasing_scale_of_even_dp() {
    assert_eq!(space::AP_DP, 2.0);
    let doc = [0.0, 2.0, 4.0, 8.0, 12.0, 16.0, 24.0, 32.0, 48.0, 64.0];
    assert_eq!(space::SCALE, doc);
    for pair in space::SCALE.windows(2) {
        assert!(pair[0] < pair[1]);
    }
    for s in space::SCALE {
        assert_eq!(s % space::AP_DP, 0.0, "{s} non è multiplo dell'AP");
    }
    assert_eq!(space::ap_to_dp(12), 24.0);
}

#[test]
fn shapes_borders_and_shadows() {
    assert_eq!(shape::RADIUS_NONE, 0.0);
    assert_eq!((shape::NOTCH_0, shape::NOTCH_1, shape::NOTCH_2), (0, 1, 2));
    assert_eq!((shape::BORDER_W, shape::FOCUS_RING_W, shape::FOCUS_RING_GAP, shape::MARKER_W), (1, 1, 1, 2));
    assert_eq!(shape::LEDGE_OFFSET, (1, 1));
    assert_eq!(shape::SHADOW_FLOAT_OFFSET, (2, 2));
    assert_eq!(shape::BLUR, 0.0);
    assert_eq!(interaction::PRESS_TRANSLATE, shape::LEDGE_OFFSET, "premuto = ledge che collassa");
    assert_ne!(shape::Corner::Square, shape::Corner::PixelCircle);
}

#[test]
fn layout_sizes_are_multiples_of_the_grid() {
    let multiples_of_4 = [
        size::TITLE_BAR_H,
        size::PLAYER_BAR_H,
        size::CONTROL_H,
        size::CONTROL_H_LARGE,
        size::ICON,
        size::ICON_BUTTON,
        size::NAV_ITEM_H,
        size::TRACK_ROW_H,
        size::TRACK_ROW_DENSE_H,
        size::MENU_ITEM_H,
        size::SEEK_TRACK_H,
        size::phone::PHONE_TOP_BAR_H,
        size::phone::PHONE_TAB_BAR_H,
        size::phone::PHONE_ROW_H,
        size::phone::SHEET_ITEM_H,
        size::phone::TOUCH_TARGET,
    ];
    for v in multiples_of_4 {
        assert_eq!(v % 4.0, 0.0, "{v}");
    }
    for v in [
        size::cover::COVER_XS,
        size::cover::COVER_S,
        size::cover::COVER_M,
        size::cover::COVER_L,
        size::cover::COVER_PANEL,
        size::cover::COVER_FOCUS_MAX,
        size::cover::AVATAR_S,
        size::cover::AVATAR_M,
        size::cover::AVATAR_L,
        size::phone::PHONE_SHELF_COVER,
    ] {
        assert_eq!(v % 16.0, 0.0, "le copertine sono multipli di 16 dp: {v}");
    }
    assert!(size::WINDOW_MIN_W > size::SIDEBAR_RAIL_W + size::SIDE_PANEL_W + size::TILE_MIN_W);
    assert!(size::BP_RAIL < size::BP_FRAME_NARROW && size::BP_FRAME_NARROW < size::BP_PANEL_DOCK);
    assert!(size::SIDEBAR_MIN_W <= size::SIDEBAR_DEFAULT_W && size::SIDEBAR_DEFAULT_W <= size::SIDEBAR_MAX_W);
    for v in size::cover::ART_RAMP {
        assert_eq!(v % 8.0, 0.0, "art sizes are whole 4 AP steps: {v}");
    }
    assert_eq!(size::cover::ART_HEADER % 8.0, 0.0);
    assert!(size::SEEK_TRACK_H < size::SEEK_HIT_H);
    assert!(size::phone::TOUCH_TARGET >= 44.0);
    assert!(size::ICON_BUTTON >= size::ICON);
}

#[test]
fn fluid_grids_fill_the_row_and_flowing_lists_take_one_to_three_columns() {
    let (columns, side) = size::tile_grid(1000.0, size::TILE_MIN, size::TILE_GAP);
    assert_eq!(columns, 5); // (1000 + 24) / 192 = 5.3
    assert!((side * 5.0 + 4.0 * size::TILE_GAP - 1000.0).abs() < 1e-3, "the tiles fill the row exactly");
    assert!(side >= size::TILE_MIN);
    assert_eq!(size::tile_grid(100.0, size::TILE_MIN, size::TILE_GAP).0, 1);
    assert_eq!(size::tile_grid(f32::NAN, size::TILE_MIN, size::TILE_GAP), (1, size::TILE_MIN));
    assert_eq!(size::flow_columns(600.0), 1);
    assert_eq!(size::flow_columns(899.0), 1);
    assert_eq!(size::flow_columns(900.0), 2);
    assert_eq!(size::flow_columns(1499.0), 2);
    assert_eq!(size::flow_columns(1500.0), 3);
    assert_eq!(size::flow_columns(f32::NAN), 1);
    assert_eq!(size::frame_margin(1100.0), size::FRAME_MARGIN_NARROW);
    assert_eq!(size::frame_margin(1200.0), size::FRAME_MARGIN);
}

#[test]
fn grid_columns_and_phone_cover_formulas() {
    assert_eq!(size::grid_columns(0.0), 1);
    assert_eq!(size::grid_columns(f32::NAN), 1);
    assert_eq!(size::grid_columns(160.0), 1);
    assert_eq!(size::grid_columns(335.0), 1);
    assert_eq!(size::grid_columns(336.0), 2); // (336 + 16) / 176
    assert_eq!(size::grid_columns(1440.0), 8);
    assert_eq!(size::phone::player_cover(360.0, 800.0), 320.0); // min(328, 336) = 328 -> 320
    assert_eq!(size::phone::player_cover(412.0, 915.0), 368.0); // min(380, 384.3) = 380 -> 368
    assert_eq!(size::phone::player_cover(400.0, 500.0), 208.0); // min(368, 210) = 210 -> 208
    assert_eq!(size::phone::player_cover(-5.0, 100.0), 0.0);
}

#[test]
fn type_scale_matches_the_doc() {
    use type_::*;
    // Desktop v2 §1: five sizes and the lyric pair.
    let ramp = [
        ("DISPLAY", FAMILY_PIXEL, 40.0, 44.0),
        ("TITLE", FAMILY_UI, 22.0, 28.0),
        ("BODY", FAMILY_UI, 15.0, 20.0),
        ("META", FAMILY_UI, 13.0, 18.0),
        ("CAPTION", FAMILY_PIXEL, 11.0, 14.0),
    ];
    for ((name, role), (doc_name, family, size, line)) in RAMP.iter().zip(ramp) {
        assert_eq!(*name, doc_name);
        assert_eq!((role.family, role.size, role.line_height), (family, size, line), "{name}");
    }
    assert_eq!((LYRIC_PANEL.size, LYRIC_PANEL.line_height), (20.0, 28.0));
    assert_eq!((LYRIC_STAGE.size, LYRIC_STAGE.line_height), (32.0, 40.0));
    assert_eq!((TITLE.weight, BODY.weight, META.weight), (600, 500, 400));
    // Every role is one size of the ramp (or of the lyric pair): nothing else is visible.
    let sizes: Vec<(f32, f32)> = RAMP
        .iter()
        .map(|(_, r)| (r.size, r.line_height))
        .chain([(LYRIC_PANEL.size, LYRIC_PANEL.line_height), (LYRIC_STAGE.size, LYRIC_STAGE.line_height)])
        .collect();
    assert_eq!(type_::ALL.len(), 15);
    for (name, role) in type_::ALL.iter() {
        assert!(
            sizes.contains(&(role.size, role.line_height)),
            "{name}: {} / {}",
            role.size,
            role.line_height
        );
        assert!(role.line_height >= role.size, "{name}: line height under the size");
        assert!([400, 500, 600].contains(&role.weight), "{name}");
    }
    assert_eq!(TYPE_HEADING, TITLE);
    assert_eq!(TYPE_TRACK_TITLE, BODY);
    assert_eq!((TYPE_BODY.size, TYPE_BODY.weight), (BODY.size, 400));
    assert_eq!((TYPE_LABEL.family, TYPE_LABEL.size), (FAMILY_PIXEL, 11.0));
    assert!(TYPE_NUMERIC_PIXEL.slashed_zero && !TYPE_NUMERIC.slashed_zero);
    assert_eq!(TYPE_LABEL.case, TextCase::Upper);
    // The pixel face: 11 / 14 AP.
    assert_eq!((PIXEL_SIZE, PIXEL_LINE), (11.0 * 2.0, 14.0 * 2.0));
    assert_eq!((PIXEL_SIZE_AP, PIXEL_LINE_AP), (11, 14));
}

#[test]
fn the_pixel_face_snaps_to_whole_multiples_of_eleven_device_pixels() {
    use type_::*;
    assert_eq!(pixel_face_device(CAPTION, 1.0), (11.0, 14.0));
    assert_eq!(pixel_face_device(CAPTION, 1.5), (22.0, 28.0), "ties round up");
    assert_eq!(pixel_face_device(CAPTION, 1.375), (11.0, 14.0));
    assert_eq!(pixel_face_device(CAPTION, 1.25 * 1.2), (22.0, 28.0), "a tie with float noise");
    assert_eq!(pixel_face_device(CAPTION, 2.0), (22.0, 28.0));
    assert_eq!(pixel_face_device(CAPTION, 0.5), (11.0, 14.0), "never under 11");
    assert_eq!(pixel_face_device(DISPLAY, 1.0), (44.0, 48.0));
    assert_eq!(pixel_face_device(DISPLAY, 1.5), (55.0, 61.0));
    assert_eq!(pixel_face_device(TYPE_WORDMARK, 1.0), (22.0, 28.0));
    assert_eq!(pixel_face_device(DISPLAY, f32::NAN).0, 11.0);
}

#[test]
fn phone_overrides_only_differ_where_the_doc_says() {
    use type_::phone;
    assert_eq!((phone::TYPE_DISPLAY.size, phone::TYPE_DISPLAY.line_height), (26.0, 32.0));
    assert_eq!((phone::TYPE_TRACK_TITLE.size, phone::TYPE_TRACK_TITLE.line_height), (16.0, 22.0));
    assert_eq!((phone::TYPE_ARTIST.size, phone::TYPE_ARTIST.line_height), (14.0, 20.0));
    assert_eq!((phone::TYPE_BODY.size, phone::TYPE_BODY.line_height), (15.0, 22.0));
    assert_eq!(
        (phone::TYPE_NAV.size, phone::TYPE_NAV.line_height, phone::TYPE_NAV.weight),
        (11.0, 16.0, 500)
    );
    assert_eq!((phone::TYPE_LYRIC.size, phone::TYPE_LYRIC.line_height), (22.0, 30.0));
    assert_eq!(phone::TYPE_TRACK_TITLE.weight, type_::TYPE_TRACK_TITLE.weight);
}

#[test]
fn motion_tokens_match_the_doc() {
    use motion::*;
    let doc = [0, 70, 100, 160, 200, 220, 280];
    for ((name, m), ms) in motion::ALL.iter().zip(doc) {
        assert_eq!(m.ms, ms, "{name}");
    }
    assert_eq!(MOTION_PRESS.curve, Curve::Step);
    assert!(motion::ALL[1..].iter().all(|(_, m)| m.curve == Curve::EaseOut));
    for pair in motion::ALL.windows(2) {
        assert!(pair[0].1.ms < pair[1].1.ms, "le durate crescono");
    }
    assert_eq!(EASE_OUT_BEZIER, [0.2, 0.0, 0.0, 1.0]);
    assert_eq!(
        (TOOLTIP_DELAY_MS, TOAST_HOLD_MS, CARET_BLINK_ON_MS, CARET_BLINK_OFF_MS, PROGRESS_TICK_MS),
        (500, 4000, 530, 530, 250)
    );
    assert_eq!(SPRITE_FRAME_MIN_MS, 80);
    assert_eq!((STAGGER_MS, STAGGER_MAX_STEPS, ENTER_RISE_AP, STEP_FRAMES), (40, 6, 4, 3));
    // reduce_motion: UI durations at 0, the progress tick is not.
    assert_eq!(MOTION_BASE.effective_ms(true), 0);
    assert_eq!(MOTION_BASE.effective_ms(false), 160);
    assert_eq!(PROGRESS_TICK_MS, 250);
    assert_eq!((interaction::LONG_PRESS_MS, interaction::DOUBLE_CLICK_MS), (450, 400));
    assert_eq!((interaction::DRAG_THRESHOLD_DESKTOP, interaction::DRAG_THRESHOLD_PHONE), (4.0, 8.0));
}

#[test]
fn z_layers_are_strictly_ordered_and_unique() {
    for pair in z::ALL.windows(2) {
        assert!(pair[0].1 < pair[1].1, "{} deve stare sotto {}", pair[0].0, pair[1].0);
    }
    assert_eq!(z::ALL.map(|(_, v)| v), [0, 10, 20, 30, 40, 50, 51, 60, 70, 100]);
    assert!(z::Z_DIALOG > z::Z_SCRIM, "il dialogo sta sopra lo scrim");
    assert!(z::Z_TOAST > z::Z_DIALOG);
}
