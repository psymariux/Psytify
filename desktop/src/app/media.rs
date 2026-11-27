//! The media slot (desktop v2 §5): one place for the record and the clip, in the side panel (280 dp) and on the
//! stage (up to 720 dp). It shows the cover, or, when the track has a clip and the clip is on, the clip at 16:9
//! in the same slot: the well of the window's single `VideoStage`, which paints the frames there. A small
//! Cover / Video switch sits on the slot when the track has a clip. Swapping faces is a 4-frame ordered-dither
//! dissolve in `MOTION_BASE` (160 ms): the new face appears through a veil that loses a quarter of its cells
//! per frame; reduced motion cuts.

use gpui::{AnyElement, App, IntoElement, ParentElement, SharedString, Styled, Window, div};
use psytify_design::tokens::motion::MOTION_BASE;
use psytify_design::tokens::space::SPACE_2;

use crate::components::now_playing::{MediaSwitch, OnToggle, veil};
use crate::components::tile::Cover;
use crate::theme::dp;

#[cfg(feature = "video")]
use crate::video::stage::{Surface, VideoStage};

/// The clip that does not open.
#[cfg(feature = "video")]
pub const FAILED: &str = "Il videoclip non si è aperto.";

/// Frames of the dissolve.
const DISSOLVE_FRAMES: u32 = 4;

/// Cells of every 4 still veiled `elapsed` ms into a dissolve: 4, 3, 2, 1, then none. Pure.
pub fn dissolve_veil(elapsed: Option<u32>) -> u8 {
    let Some(elapsed) = elapsed else { return 0 };
    let frame = (elapsed * DISSOLVE_FRAMES / MOTION_BASE.ms.max(1)).min(DISSOLVE_FRAMES);
    u8::try_from(DISSOLVE_FRAMES - frame).unwrap_or(0)
}

/// The height of the slot for its width: a square for the cover, 16:9 for the clip.
pub fn slot_height(width: f32, video: bool) -> f32 {
    if video { (width * 9.0 / 16.0).round() } else { width }
}

/// What a media slot shows.
pub struct Slot {
    /// The element id prefix (the dissolve and the switch keep their state under it).
    pub id: &'static str,
    /// The slot width in dp (the cover's side, the clip's width).
    pub width: f32,
    pub cover_url: Option<SharedString>,
    /// The whole cover image (the stage).
    pub large: bool,
    /// The track has a clip (the switch shows).
    pub has_video: bool,
    /// The clip is on (and the track has one).
    pub video: bool,
    pub on_toggle: OnToggle,
}

/// The slot with its switch; with the `video` feature the clip face is the stage's well in `surface`.
pub fn media_slot(
    slot: Slot,
    #[cfg(feature = "video")] stage: Option<(&gpui::Entity<VideoStage>, Surface)>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let elapsed = crate::motion::since_change(
        gpui::ElementId::Name(format!("{}-face", slot.id).into()),
        slot.video,
        MOTION_BASE.ms,
        window,
        cx,
    );
    let covered = dissolve_veil(elapsed);
    let height = slot_height(slot.width, slot.video);
    let face: AnyElement = match () {
        #[cfg(feature = "video")]
        () if slot.video && stage.is_some() => {
            let Some((stage, surface)) = stage else { return div().into_any_element() };
            let scale = window.scale_factor() * crate::theme::zoom();
            stage.update(cx, |stage, _| {
                stage.set_surface_px(crate::video::size::device(slot.width, height, scale));
            });
            stage.read(cx).well(surface, slot.width, height, covered).into_any_element()
        }
        () => {
            let mut cover = Cover::new(slot.width).url(slot.cover_url.clone());
            if slot.large {
                cover = cover.large();
            }
            div().relative().child(cover).child(veil(covered)).into_any_element()
        }
    };
    let mut root = div().relative().flex_none().w(dp(slot.width)).h(dp(height)).child(face);
    if slot.has_video {
        let toggle = slot.on_toggle.clone();
        root = root.child(
            div().absolute().top(dp(SPACE_2)).right(dp(SPACE_2)).child(
                MediaSwitch::new(format!("{}-switch", slot.id), slot.video)
                    .on_toggle(move |video, window, cx| toggle(video, window, cx)),
            ),
        );
    }
    root.into_any_element()
}

/// The line under the clip: waiting, or the failure with «Riprova».
#[cfg(feature = "video")]
pub fn video_status(stage: &gpui::Entity<VideoStage>, id: &'static str, cx: &App) -> Option<AnyElement> {
    use psytify_design::tokens::color::TEXT_SECONDARY;
    use psytify_design::tokens::space::SPACE_4;
    use psytify_design::tokens::type_::TYPE_ARTIST;

    use crate::components::button::Button;
    use crate::components::indicators::LoadingIndicator;
    use crate::theme::{TypeStyle, col};
    use crate::video::stage::Status;

    match stage.read(cx).status() {
        Status::Loading => Some(LoadingIndicator::new().caption("Preparo il videoclip…").into_any_element()),
        Status::Failed => {
            let stage = stage.clone();
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap(dp(SPACE_4))
                    .child(div().type_role(TYPE_ARTIST).text_color(col(TEXT_SECONDARY)).child(FAILED))
                    .child(
                        Button::new(SharedString::new_static(id), "Riprova")
                            .secondary()
                            .on_click(move |_, _, cx| stage.read(cx).retry(cx)),
                    )
                    .into_any_element(),
            )
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dissolve_uncovers_a_quarter_per_frame_then_rests() {
        assert_eq!(dissolve_veil(None), 0);
        assert_eq!(dissolve_veil(Some(0)), 4);
        assert_eq!(dissolve_veil(Some(40)), 3);
        assert_eq!(dissolve_veil(Some(80)), 2);
        assert_eq!(dissolve_veil(Some(159)), 1);
        assert_eq!(dissolve_veil(Some(160)), 0);
    }

    #[test]
    fn the_clip_takes_sixteen_by_nine_of_the_slot() {
        assert_eq!(slot_height(280.0, false), 280.0);
        assert_eq!(slot_height(280.0, true), 158.0);
        assert_eq!(slot_height(640.0, true), 360.0);
    }
}
