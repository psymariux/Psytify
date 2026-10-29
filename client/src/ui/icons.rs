//! Icone SVG su 24px, colorate con il filtro di Iced. Ogni handle viene creato una sola volta.
//! Le icone di interfaccia sono Iconly (Light e Bold, vedi `assets/icons/CREDITS.md`); i controlli
//! multimediali che il set gratuito non ha (pausa, successivo, casuale, ripeti...) sono disegnati
//! con lo stesso tratto da 1.5px e le estremità tonde.

use std::sync::LazyLock;

use iced::widget::svg::{Handle, Svg};
use iced::{Color, Length};

use super::theme;

macro_rules! icons {
    ($($name:ident => $file:literal),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Icon { $($name),* }

        impl Icon {
            fn handle(self) -> Handle {
                $(
                    #[allow(non_upper_case_globals)]
                    static $name: LazyLock<Handle> = LazyLock::new(|| {
                        Handle::from_memory(include_bytes!(concat!("../../assets/icons/", $file)).as_slice())
                    });
                )*
                match self { $(Self::$name => $name.clone()),* }
            }
        }
    };
}

icons! {
    Home => "home.svg",
    HomeFill => "home-fill.svg",
    Search => "search.svg",
    SearchFill => "search-fill.svg",
    Library => "library.svg",
    LibraryFill => "library-fill.svg",
    Play => "play.svg",
    Pause => "pause.svg",
    Next => "next.svg",
    Previous => "previous.svg",
    Shuffle => "shuffle.svg",
    Repeat => "repeat.svg",
    RepeatOne => "repeat-one.svg",
    Queue => "queue.svg",
    Lyrics => "lyrics.svg",
    Equalizer => "equalizer.svg",
    People => "people.svg",
    PeopleFill => "people-fill.svg",
    Account => "account.svg",
    Volume => "volume.svg",
    VolumeLow => "volume-low.svg",
    Mute => "mute.svg",
    Download => "download.svg",
    Plus => "plus.svg",
    Close => "close.svg",
    List => "list.svg",
    Mark => "mark.svg",
    Settings => "settings.svg",
    SettingsFill => "settings-fill.svg",
    Video => "video.svg",
    Heart => "heart.svg",
    HeartFill => "heart-fill.svg",
    More => "more.svg",
    Trash => "trash.svg",
    Edit => "edit.svg",
    Check => "check.svg",
    ChevronRight => "chevron-right.svg",
    ChevronLeft => "chevron-left.svg",
    ChevronDown => "chevron-down.svg",
    Keyboard => "keyboard.svg",
    Minimize => "minimize.svg",
    Fullscreen => "fullscreen.svg",
    FullscreenExit => "fullscreen-exit.svg",
    Note => "note.svg",
    Bell => "bell.svg",
    BellFill => "bell-fill.svg",
    Moon => "moon.svg",
    Expand => "expand.svg",
    Copy => "copy.svg",
    Radio => "radio.svg",
    Sparkle => "sparkle.svg",
    UserPlus => "user-plus.svg",
    Chart => "chart.svg",
    Google => "brand-google.svg",
    GitHub => "brand-github.svg",
    Discord => "brand-discord.svg",
}

/// Icona tinta nel colore indicato.
pub fn icon<'a>(icon: Icon, size: f32, color: Color) -> Svg<'a> {
    Svg::new(icon.handle())
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(theme::icon_tint(color))
}

/// Il marchio mantiene i suoi colori: non viene tinto.
pub fn mark<'a>(size: f32) -> Svg<'a> {
    Svg::new(Icon::Mark.handle()).width(Length::Fixed(size)).height(Length::Fixed(size))
}
