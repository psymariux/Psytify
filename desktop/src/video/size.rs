//! How big to decode: GStreamer reduces the video to the physical pixels of the box that shows it
//! (`videoscale`), so the mini player in the panel neither copies nor uploads to the atlas the 3.7 MB of a
//! 720p frame to draw a tenth of it. Never larger than the video (enlarging is done by the GPU
//! when it draws, for free) nor 1280 x 720 (`05-video-spike.md` §5: no 1080p).

/// The largest frame asked of GStreamer.
pub const MAX_W: u32 = 1280;
pub const MAX_H: u32 = 720;

/// A width change below this fraction is not worth a new negotiation with GStreamer (while
/// the window is resized the measure changes at every step).
const TOLERANCE: f32 = 1.0 / 16.0;

/// Una misura in pixel fisici.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// The measure to ask for a video of `native` pixels in a box of `surface` physical pixels:
/// inside the box, with the video's proportions, never larger than the video nor
/// `MAX_W` x `MAX_H`, even sides (the upstream YUV formats want them). `None` if one of the two measures is
/// empty: then it decodes at the video's measure.
pub fn fit(native: Size, surface: Size) -> Option<Size> {
    if native.width == 0 || native.height == 0 || surface.width == 0 || surface.height == 0 {
        return None;
    }
    let (nw, nh) = (native.width as f32, native.height as f32);
    let scale = (surface.width as f32 / nw)
        .min(surface.height as f32 / nh)
        .min(MAX_W as f32 / nw)
        .min(MAX_H as f32 / nh)
        .min(1.0);
    // Rounded down (never beyond the box), with a margin for the floats' rounding.
    let even = |value: f32| (((value + 0.01) / 2.0).floor() as u32 * 2).max(2);
    Some(Size::new(even(nw * scale), even(nh * scale)))
}

/// GStreamer is asked for a new measure only if the one in use is smaller than the one needed (the
/// frame would look grainy) or larger by more than `TOLERANCE` (work would be wasted).
pub fn worth_switching(current: Size, wanted: Size) -> bool {
    if wanted.width > current.width || wanted.height > current.height {
        return true;
    }
    (current.width - wanted.width) as f32 > current.width as f32 * TOLERANCE
}

/// A box of `width` x `height` dp in physical pixels, at the window's scale.
pub fn device(width: f32, height: f32, scale: f32) -> Size {
    let px = |value: f32| (value * scale).round().max(0.0) as u32;
    Size::new(px(width), px(height))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HD: Size = Size::new(1280, 720);
    const SD: Size = Size::new(640, 360);

    #[test]
    fn the_mini_player_gets_its_own_pixels_and_never_more_than_the_clip() {
        // The panel's mini player (288 x 162 dp): at scale 1 a tenth of 720p, at scale 2 double.
        assert_eq!(fit(HD, device(288.0, 162.0, 1.0)), Some(Size::new(288, 162)));
        assert_eq!(fit(HD, device(288.0, 162.0, 2.0)), Some(Size::new(576, 324)));
        // The 360p in the mini player at scale 2 shrinks a little; in a larger box it is not enlarged.
        assert_eq!(fit(SD, device(288.0, 162.0, 2.0)), Some(Size::new(576, 324)));
        assert_eq!(fit(SD, device(640.0, 360.0, 2.0)), Some(SD));
    }

    #[test]
    fn the_focus_view_is_capped_at_720p_and_keeps_the_clip_proportions() {
        // Una vista grande a scala 2 vorrebbe 1080p e oltre: si ferma al videoclip e a 1280 x 720.
        assert_eq!(fit(HD, device(1600.0, 900.0, 2.0)), Some(HD));
        assert_eq!(fit(Size::new(1920, 1080), device(1600.0, 900.0, 2.0)), Some(HD));
        // A 4:3 video in a 16:9 box fits the height; a box taller than wide.
        assert_eq!(fit(Size::new(960, 720), Size::new(1280, 360)), Some(Size::new(480, 360)));
        assert_eq!(fit(HD, Size::new(320, 900)), Some(Size::new(320, 180)));
        // Even sides, never beyond the box, never zero.
        let odd = fit(Size::new(853, 480), Size::new(301, 301)).unwrap_or(HD);
        assert_eq!((odd.width % 2, odd.height % 2), (0, 0));
        assert!(odd.width <= 301 && odd.height <= 301);
        assert_eq!(fit(HD, Size::new(1, 1)), Some(Size::new(2, 2)));
    }

    #[test]
    fn an_empty_measure_decodes_at_the_clip_size() {
        assert_eq!(fit(Size::new(0, 0), HD), None);
        assert_eq!(fit(HD, Size::new(0, 162)), None);
    }

    #[test]
    fn small_changes_do_not_renegotiate_but_a_bigger_box_does() {
        let mini = Size::new(288, 162);
        assert!(!worth_switching(mini, mini));
        // A few pixels less while resizing: not worth a negotiation.
        assert!(!worth_switching(mini, Size::new(280, 158)));
        // Much smaller: yes. Larger even a little: yes (it would look grainy).
        assert!(worth_switching(HD, mini));
        assert!(worth_switching(mini, Size::new(290, 164)));
    }
}
