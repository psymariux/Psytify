//! Icone e sprite pixel art di `psytify-design`, disegnati nitidi: lo sprite 1x si ingrandisce su CPU
//! di un fattore intero (`scale::art_px(fattore di scala)`, un pixel d'arte è un blocco di pixel
//! fisici) e Iced lo disegna 1:1 con il filtro `Nearest`. Così i bordi restano netti a ogni scala,
//! cosa che un SVG o una texture ingrandita dalla GPU non garantiscono.
//!
//! Il lavoro (ingrandire e tingere) si fa una sola volta per `(sprite, frame, fattore, tinta)` e sta in
//! una cache con limite (`UpscaleCache`: 128 voci, 8 MiB) più la mappa degli `image::Handle` (Iced dà
//! a ogni handle un identificatore nuovo: va conservato, non ricreato a ogni vista). La vista fa solo
//! una ricerca. Quando cambia il fattore di scala si svuota tutto.
//!
//! Il fattore di scala della finestra lo comunica l'app con [`set_scale_factor`]: Iced 0.13 non lo dà
//! alla vista. Finché non arriva si usa 1.
//!
//! Gli unici SVG restano i marchi dei provider di accesso (Google, GitHub, Discord), che non si
//! ridisegnano: sono l'eccezione dichiarata nella direzione artistica.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{LazyLock, Mutex, PoisonError};

use iced::widget::image::{FilterMethod, Handle as ImageHandle};
use iced::widget::svg::{Handle as SvgHandle, Svg};
use iced::widget::{container, image, svg};
use iced::{Color, ContentFit, Element, Length};
use psytify_design::scale::art_px;
use psytify_design::sprite::{self, UpscaleCache};
use psytify_design::tokens::color::Rgba;

use super::theme;

/// Uno sprite dell'insieme di icone (o un marchio di provider). Il nome dice a che cosa serve: lo
/// sprite lo sceglie `Icon::sprite`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Home,
    HomeFill,
    Search,
    SearchFill,
    Library,
    LibraryFill,
    Play,
    Pause,
    Next,
    Previous,
    Shuffle,
    Repeat,
    RepeatOne,
    Queue,
    Lyrics,
    Equalizer,
    People,
    PeopleFill,
    Account,
    Volume,
    VolumeLow,
    Mute,
    Download,
    /// Il brano è sul dispositivo (il marcatore di «scaricato»).
    Downloaded,
    Plus,
    Close,
    List,
    Settings,
    SettingsFill,
    Video,
    Heart,
    HeartFill,
    More,
    Trash,
    Edit,
    Check,
    ChevronRight,
    ChevronLeft,
    ChevronDown,
    Keyboard,
    Minimize,
    Fullscreen,
    FullscreenExit,
    Note,
    Bell,
    BellFill,
    Moon,
    /// La vista immersiva.
    Expand,
    Copy,
    Radio,
    Sparkle,
    UserPlus,
    Chart,
    /// I marchi dei provider di accesso: vettoriali, con i loro colori.
    Google,
    GitHub,
    Discord,
}

impl Icon {
    /// Il nome dello sprite, o `None` per i marchi dei provider.
    fn sprite(self) -> Option<&'static str> {
        use sprite::*;
        Some(match self {
            Self::Home => ICON_HOME,
            Self::HomeFill => ICON_HOME_FILL,
            Self::Search => ICON_SEARCH,
            Self::SearchFill => ICON_SEARCH_FILL,
            Self::Library => ICON_LIBRARY,
            Self::LibraryFill => ICON_LIBRARY_FILL,
            Self::Play => ICON_PLAY,
            Self::Pause => ICON_PAUSE,
            Self::Next => ICON_NEXT,
            Self::Previous => ICON_PREVIOUS,
            Self::Shuffle => ICON_SHUFFLE,
            Self::Repeat => ICON_REPEAT,
            Self::RepeatOne => ICON_REPEAT_ONE,
            Self::Queue => ICON_QUEUE,
            Self::Lyrics => ICON_LYRICS,
            Self::Equalizer => ICON_EQUALIZER,
            Self::People => ICON_PEOPLE,
            Self::PeopleFill => ICON_PEOPLE_FILL,
            Self::Account => ICON_ACCOUNT,
            Self::Volume => ICON_VOLUME,
            Self::VolumeLow => ICON_VOLUME_LOW,
            Self::Mute => ICON_MUTE,
            Self::Download => ICON_DOWNLOAD,
            Self::Downloaded => ICON_DOWNLOADED,
            Self::Plus => ICON_PLUS,
            Self::Close => ICON_CLOSE,
            Self::List => ICON_LIST,
            Self::Settings => ICON_SETTINGS,
            Self::SettingsFill => ICON_SETTINGS_FILL,
            Self::Video => ICON_VIDEO,
            Self::Heart => ICON_HEART,
            Self::HeartFill => ICON_HEART_FILL,
            Self::More => ICON_MORE,
            Self::Trash => ICON_TRASH,
            Self::Edit => ICON_EDIT,
            Self::Check => ICON_CHECK,
            Self::ChevronRight => ICON_CHEVRON_RIGHT,
            Self::ChevronLeft => ICON_CHEVRON_LEFT,
            Self::ChevronDown => ICON_CHEVRON_DOWN,
            Self::Keyboard => ICON_KEYBOARD,
            Self::Minimize => ICON_MINIMIZE,
            Self::Fullscreen => ICON_FULLSCREEN,
            Self::FullscreenExit => ICON_FULLSCREEN_EXIT,
            Self::Note => ICON_NOTE,
            Self::Bell => ICON_BELL,
            Self::BellFill => ICON_BELL_FILL,
            Self::Moon => ICON_MOON,
            Self::Expand => ICON_FOCUS,
            Self::Copy => ICON_COPY,
            Self::Radio => ICON_RADIO,
            Self::Sparkle => ICON_SPARKLE,
            Self::UserPlus => ICON_USER_PLUS,
            Self::Chart => ICON_CHART,
            Self::Google | Self::GitHub | Self::Discord => return None,
        })
    }

    /// Il marchio vettoriale di un provider.
    fn brand(self) -> Option<SvgHandle> {
        static GOOGLE: LazyLock<SvgHandle> = LazyLock::new(|| {
            SvgHandle::from_memory(include_bytes!("../../assets/icons/brand-google.svg").as_slice())
        });
        static GITHUB: LazyLock<SvgHandle> = LazyLock::new(|| {
            SvgHandle::from_memory(include_bytes!("../../assets/icons/brand-github.svg").as_slice())
        });
        static DISCORD: LazyLock<SvgHandle> = LazyLock::new(|| {
            SvgHandle::from_memory(include_bytes!("../../assets/icons/brand-discord.svg").as_slice())
        });
        match self {
            Self::Google => Some(GOOGLE.clone()),
            Self::GitHub => Some(GITHUB.clone()),
            Self::Discord => Some(DISCORD.clone()),
            _ => None,
        }
    }
}

// ---------- Cache degli sprite ingranditi ----------

/// Voci della mappa degli handle: oltre, si svuota (si ricostruisce da `UpscaleCache`, già pronta).
const MAX_HANDLES: usize = 192;

type HandleKey = (usize, usize, u32, Option<Rgba>);

struct SpriteCache {
    pixels: UpscaleCache,
    handles: HashMap<HandleKey, ImageHandle>,
}

static CACHE: LazyLock<Mutex<SpriteCache>> =
    LazyLock::new(|| Mutex::new(SpriteCache { pixels: UpscaleCache::new(), handles: HashMap::new() }));

/// Il fattore di scala della finestra (bit di un `f32`), 1.0 finché l'app non lo comunica.
static SCALE_BITS: AtomicU32 = AtomicU32::new(0x3f80_0000);

/// Il fattore di scala della finestra, come lo ha comunicato l'app.
pub fn scale_factor() -> f32 {
    let value = f32::from_bits(SCALE_BITS.load(Ordering::Relaxed));
    if value.is_finite() && value > 0.0 { value } else { 1.0 }
}

/// Comunica il fattore di scala della finestra. Vero se è cambiato il pixel d'arte (da qui in poi gli
/// sprite si ingrandiscono di un altro fattore intero): la cache si svuota, la vista la riempie.
pub fn set_scale_factor(factor: f32) -> bool {
    let before = art_px(scale_factor());
    SCALE_BITS.store(factor.to_bits(), Ordering::Relaxed);
    let changed = before != art_px(factor);
    if changed {
        let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        cache.pixels.clear();
        cache.handles.clear();
    }
    changed
}

/// L'immagine di un frame di uno sprite, ingrandita del pixel d'arte corrente e (per le maschere)
/// tinta. `None` se lo sprite o il frame non esistono (asset corrotti: bug di build).
fn handle_of(name: &str, frame: usize, tint: Option<Rgba>, zoom: u32) -> Option<(ImageHandle, u32, u32)> {
    let sheet = sprite::sheet().ok()?;
    let sprite = sheet.get(name)?;
    let factor = (art_px(scale_factor()) * zoom.max(1)).min(sprite::MAX_FACTOR);
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    let pixels = cache.pixels.get_or_upscale(sprite, frame, factor, tint)?;
    let tint = if sprite.kind() == sprite::SpriteKind::Mask { tint } else { None };
    let key = (sprite.index(), frame, factor, tint);
    if cache.handles.len() >= MAX_HANDLES && !cache.handles.contains_key(&key) {
        cache.handles.clear();
    }
    let handle = cache
        .handles
        .entry(key)
        .or_insert_with(|| ImageHandle::from_rgba(pixels.width, pixels.height, pixels.rgba.clone()))
        .clone();
    Some((handle, pixels.width, pixels.height))
}

/// Lo sprite `name` al frame `frame`, grande `zoom` volte il pixel d'arte, tinto con `tint` se è una
/// maschera. Le dimensioni sono esatte (pixel fisici diviso fattore di scala) e il filtro è `Nearest`.
pub fn sprite_image<'a, M: 'a>(name: &str, frame: usize, tint: Option<Color>, zoom: u32) -> Element<'a, M> {
    let Some((handle, w, h)) = handle_of(name, frame, tint.map(theme::rgba_of), zoom) else {
        return container(iced::widget::Space::new(0, 0)).into();
    };
    let scale = scale_factor();
    let mut picture = image(handle)
        .width(Length::Fixed(w as f32 / scale))
        .height(Length::Fixed(h as f32 / scale))
        .content_fit(ContentFit::Fill)
        .filter_method(FilterMethod::Nearest);
    if let Some(color) = tint.filter(|c| c.a < 1.0) {
        picture = picture.opacity(color.a);
    }
    picture.into()
}

/// Il frame di `name` in scena `elapsed_ms` dopo l'inizio: la vista lo chiede a ogni disegno, quindi
/// gli sprite animati avanzano al ritmo dei messaggi dell'app (il Tick mentre suona), senza timer.
pub fn frame_at(name: &str, elapsed_ms: u64) -> usize {
    sprite::sheet().ok().and_then(|sheet| sheet.get(name)).map_or(0, |s| s.frame_index_at(elapsed_ms))
}

/// Come [`frame_at`], sull'orologio di sistema: per gli sprite che girano finché sono in scena (`eq`).
pub fn frame_now(name: &str) -> usize {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(0));
    frame_at(name, ms)
}

/// Lato di uno sprite di `art_pixels` pixel d'arte, in dp, al fattore di scala corrente (non ha mai
/// più di un AP di scarto dal nominale: tabella 2 dei token).
pub fn sprite_dp(art_pixels: u32) -> f32 {
    (art_px(scale_factor()) * art_pixels) as f32 / scale_factor()
}

/// Icona nel colore indicato. `size` è il lato dello spazio che occupa: l'icona (12 AP, circa 24 dp)
/// ci sta al centro, mai ingrandita; uno spazio più piccolo dell'icona vale l'icona. Il colore sceglie
/// il token per lo stato (attiva, a riposo, disabilitata); un alfa sotto 1 diventa opacità dell'immagine.
pub fn icon<'a, M: 'a>(icon: Icon, size: f32, color: Color) -> Element<'a, M> {
    let Some(name) = icon.sprite() else {
        return match icon.brand() {
            Some(handle) => brand(handle, size, color).into(),
            None => container(iced::widget::Space::new(0, 0)).into(),
        };
    };
    let natural = sprite_dp(12);
    let picture = sprite_image(name, 0, Some(color), 1);
    if size > natural + 0.5 { container(picture).center(Length::Fixed(size)).into() } else { picture }
}

/// Il marchio di un provider: vettoriale, nel suo colore (quelli di GitHub e Discord sono neri nel
/// file e si tingono con `color`, come i marchi monocromatici che le linee guida dei provider ammettono).
fn brand<'a>(handle: SvgHandle, size: f32, color: Color) -> Svg<'a> {
    Svg::new(handle)
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_, _| svg::Style { color: Some(color) })
}

/// Il marchio dell'app: la faccia di Nastro in un colore. A riposo è bianco caldo; mentre suona la
/// musica passa a `mark_play` in lime, la luce di riproduzione del marchio.
pub fn mark<'a, M: 'a>(playing: bool) -> Element<'a, M> {
    if playing {
        sprite_image(sprite::MARK_PLAY, 0, Some(theme::LIME), 1)
    } else {
        sprite_image(sprite::MARK, 0, Some(theme::TEXT), 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_has_an_embedded_sprite_except_the_provider_marks() {
        let sheet = sprite::sheet().expect("sprite sheet");
        for icon in [Icon::Home, Icon::HeartFill, Icon::Expand, Icon::Chart, Icon::UserPlus] {
            assert!(icon.sprite().and_then(|n| sheet.get(n)).is_some(), "{icon:?}");
        }
        assert!(Icon::Google.sprite().is_none() && Icon::Google.brand().is_some());
    }

    #[test]
    fn a_sprite_is_upscaled_by_a_whole_number_and_cached() {
        let (first, w, h) = handle_of(sprite::ICON_PLAY, 0, Some(Rgba::rgb(255, 255, 255)), 1).expect("play");
        let factor = art_px(scale_factor());
        assert_eq!((w, h), (12 * factor, 12 * factor));
        // La seconda richiesta restituisce lo stesso handle (stesso identificatore): niente rifatto.
        let (second, ..) = handle_of(sprite::ICON_PLAY, 0, Some(Rgba::rgb(255, 255, 255)), 1).expect("play");
        assert_eq!(first.id(), second.id());
        // Un'altra tinta è un'altra voce.
        let (other, ..) = handle_of(sprite::ICON_PLAY, 0, Some(Rgba::rgb(198, 242, 78)), 1).expect("play");
        assert_ne!(first.id(), other.id());
    }
}
