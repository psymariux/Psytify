//! Elenco degli sprite incorporati: una costante per nome (`sprite::ICON_HEART_FILL`) e la tabella dei
//! PNG 1x compilati nel binario con `include_bytes!`. Un solo elenco alimenta entrambe le cose; un test
//! lo confronta con `sprites.json` (aggiungere uno sprite = una riga qui, dopo `sprites.py`).

macro_rules! embedded_sprites {
    ($($konst:ident => $name:literal,)*) => {
        $(
            #[doc = concat!("Nome dello sprite `", $name, "`.")]
            pub const $konst: &str = $name;
        )*

        /// Nome e byte del PNG indicizzato 1x di ogni sprite.
        pub(super) const EMBEDDED_PNGS: &[(&str, &[u8])] = &[
            $((
                $name,
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/assets/pixel/export/1x/",
                    $name,
                    ".png"
                )),
            ),)*
        ];

        /// Tutti i nomi, nell'ordine di `sprites.json`.
        pub const ALL_NAMES: &[&str] = &[$($name,)*];
    };
}

embedded_sprites! {
    ICON_PLAY => "icon_play",
    ICON_PAUSE => "icon_pause",
    ICON_PLAY_PAUSE => "icon_play_pause",
    ICON_NEXT => "icon_next",
    ICON_PREVIOUS => "icon_previous",
    ICON_SHUFFLE => "icon_shuffle",
    ICON_REPEAT => "icon_repeat",
    ICON_REPEAT_ONE => "icon_repeat_one",
    ICON_HEART => "icon_heart",
    ICON_HEART_FILL => "icon_heart_fill",
    ICON_HEART_HALF => "icon_heart_half",
    ICON_DOWNLOAD => "icon_download",
    ICON_DOWNLOADED => "icon_downloaded",
    ICON_QUEUE => "icon_queue",
    ICON_SEARCH => "icon_search",
    ICON_HOME => "icon_home",
    ICON_LIBRARY => "icon_library",
    ICON_MORE => "icon_more",
    ICON_CLOSE => "icon_close",
    ICON_CHECK => "icon_check",
    ICON_CHEVRON_LEFT => "icon_chevron_left",
    ICON_CHEVRON_RIGHT => "icon_chevron_right",
    ICON_CHEVRON_DOWN => "icon_chevron_down",
    ICON_PLUS => "icon_plus",
    ICON_VOLUME => "icon_volume",
    ICON_VOLUME_LOW => "icon_volume_low",
    ICON_MUTE => "icon_mute",
    ICON_LYRICS => "icon_lyrics",
    ICON_FOCUS => "icon_focus",
    ICON_FULLSCREEN => "icon_fullscreen",
    ICON_FULLSCREEN_EXIT => "icon_fullscreen_exit",
    ICON_MINIMIZE => "icon_minimize",
    ICON_MAXIMIZE => "icon_maximize",
    ICON_WINDOW_CLOSE => "icon_window_close",
    ICON_SETTINGS => "icon_settings",
    ICON_SETTINGS_FILL => "icon_settings_fill",
    ICON_ACCOUNT => "icon_account",
    ICON_PEOPLE => "icon_people",
    ICON_PEOPLE_FILL => "icon_people_fill",
    ICON_BELL => "icon_bell",
    ICON_BELL_FILL => "icon_bell_fill",
    ICON_RADIO => "icon_radio",
    ICON_EDIT => "icon_edit",
    ICON_TRASH => "icon_trash",
    ICON_COPY => "icon_copy",
    ICON_KEYBOARD => "icon_keyboard",
    ICON_VIDEO => "icon_video",
    ICON_GRIP => "icon_grip",
    ICON_HOME_FILL => "icon_home_fill",
    ICON_SEARCH_FILL => "icon_search_fill",
    ICON_LIBRARY_FILL => "icon_library_fill",
    ICON_LIST => "icon_list",
    ICON_NOTE => "icon_note",
    ICON_MOON => "icon_moon",
    ICON_SPARKLE => "icon_sparkle",
    ICON_CHART => "icon_chart",
    ICON_EQUALIZER => "icon_equalizer",
    ICON_USER_PLUS => "icon_user_plus",
    ICON_BELL_RING => "icon_bell_ring",
    ICON_DOWNLOAD_BUSY => "icon_download_busy",
    ICON_PEOPLE_LIVE => "icon_people_live",
    NASTRO_CHEER => "nastro_cheer",
    EQ => "eq",
    EQ_REST => "eq_rest",
    MARK => "mark",
    MARK_PLAY => "mark_play",
    LOADER => "loader",
    ALBUM_PLACEHOLDER => "album_placeholder",
    NASTRO_IDLE => "nastro_idle",
    NASTRO_LISTEN => "nastro_listen",
    NASTRO_SLEEP => "nastro_sleep",
    EMPTY_CRATE => "empty_crate",
    LIKED_COVER => "liked_cover",
    APP_ICON => "app_icon",
}
