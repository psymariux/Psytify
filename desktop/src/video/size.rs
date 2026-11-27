//! Quanto grande decodificare: GStreamer riduce il videoclip ai pixel fisici del riquadro che lo mostra
//! (`videoscale`), così il mini player nel pannello non copia né carica nell'atlante i 3,7 MB di un
//! fotogramma a 720p per disegnarne un decimo. Mai più grande del videoclip (ingrandire lo fa la GPU
//! quando disegna, gratis) né di 1280 x 720 (`05-video-spike.md` §5: niente 1080p).

/// Il fotogramma più grande che si chiede a GStreamer.
pub const MAX_W: u32 = 1280;
pub const MAX_H: u32 = 720;

/// Uno scarto di larghezza sotto questa frazione non vale una nuova negoziazione con GStreamer (mentre
/// si ridimensiona la finestra la misura cambia a ogni passo).
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

/// La misura da chiedere per un videoclip di `native` pixel in un riquadro di `surface` pixel fisici:
/// dentro il riquadro, con le proporzioni del videoclip, mai più grande del videoclip né di
/// `MAX_W` x `MAX_H`, lati pari (lo vogliono i formati YUV a monte). `None` se una delle due misure è
/// vuota: allora si decodifica alla misura del videoclip.
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
    // Per difetto (mai oltre il riquadro), con un margine per gli arrotondamenti dei float.
    let even = |value: f32| (((value + 0.01) / 2.0).floor() as u32 * 2).max(2);
    Some(Size::new(even(nw * scale), even(nh * scale)))
}

/// Si chiede a GStreamer una misura nuova solo se quella in uso è più piccola di quella che serve (il
/// fotogramma si vedrebbe sgranato) o più grande di oltre `TOLERANCE` (si sprecherebbe lavoro).
pub fn worth_switching(current: Size, wanted: Size) -> bool {
    if wanted.width > current.width || wanted.height > current.height {
        return true;
    }
    (current.width - wanted.width) as f32 > current.width as f32 * TOLERANCE
}

/// Un riquadro di `width` x `height` dp in pixel fisici, alla scala della finestra.
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
        // Il mini player del pannello (288 x 162 dp): a scala 1 un decimo del 720p, a scala 2 il doppio.
        assert_eq!(fit(HD, device(288.0, 162.0, 1.0)), Some(Size::new(288, 162)));
        assert_eq!(fit(HD, device(288.0, 162.0, 2.0)), Some(Size::new(576, 324)));
        // Il 360p nel mini player a scala 2 si riduce appena; in un riquadro più grande non si ingrandisce.
        assert_eq!(fit(SD, device(288.0, 162.0, 2.0)), Some(Size::new(576, 324)));
        assert_eq!(fit(SD, device(640.0, 360.0, 2.0)), Some(SD));
    }

    #[test]
    fn the_focus_view_is_capped_at_720p_and_keeps_the_clip_proportions() {
        // Una vista grande a scala 2 vorrebbe 1080p e oltre: si ferma al videoclip e a 1280 x 720.
        assert_eq!(fit(HD, device(1600.0, 900.0, 2.0)), Some(HD));
        assert_eq!(fit(Size::new(1920, 1080), device(1600.0, 900.0, 2.0)), Some(HD));
        // Un videoclip 4:3 in un riquadro 16:9 si adatta all'altezza; un riquadro alto alla larghezza.
        assert_eq!(fit(Size::new(960, 720), Size::new(1280, 360)), Some(Size::new(480, 360)));
        assert_eq!(fit(HD, Size::new(320, 900)), Some(Size::new(320, 180)));
        // Lati sempre pari, mai oltre il riquadro, mai zero.
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
        // Qualche pixel in meno mentre si ridimensiona: non vale una negoziazione.
        assert!(!worth_switching(mini, Size::new(280, 158)));
        // Molto più piccolo: sì. Più grande anche di poco: sì (si vedrebbe sgranato).
        assert!(worth_switching(HD, mini));
        assert!(worth_switching(mini, Size::new(290, 164)));
    }
}
