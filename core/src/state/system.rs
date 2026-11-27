//! Ciò che il sistema e Discord mostrano del brano in corso (passo 4.3b): il widget multimediale
//! del desktop (MPRIS), la schermata di blocco del telefono e «Sta ascoltando» su Discord.
//!
//! Il riduttore decide **che cosa** mostrare e **quando**, guardando la vista della riproduzione:
//! solo a un passaggio (un altro brano, pausa e ripresa, un salto nel brano), mai a ogni battito;
//! l'host, che tiene i collegamenti col sistema sul thread del core, esegue gli effetti
//! ([`MediaOp`], [`DiscordOp`]). La posizione da mostrare è `clock.at(now)` con l'`now` iniettato,
//! così il riduttore resta senza orologio; Discord vuole il tempo del sistema e lo prende l'host.

use std::time::{Duration, Instant};

use bytes::Bytes;
use uuid::Uuid;

use crate::config::DEFAULT_DISCORD_APP_ID;
use crate::effect::Effect;
use crate::media::Now;
use crate::state::playback::PlaybackView;

/// La copertina già decodificata, che il frontend consegna al core con
/// [`PlaybackCmd::Artwork`](crate::command::PlaybackCmd::Artwork): la sessione multimediale di
/// Android la vuole in pixel, mentre MPRIS legge l'indirizzo. Il core la tiene finché il brano in
/// corso è quello della sua `url`.
#[derive(Clone)]
pub struct Artwork {
    pub url: String,
    pub side: u32,
    pub rgba: Bytes,
}

impl std::fmt::Debug for Artwork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Artwork").field("url", &self.url).field("side", &self.side).finish_non_exhaustive()
    }
}

/// Un'operazione sul collegamento multimediale del sistema. L'host la esegue se il collegamento è
/// pronto (`Event::Ready`); prima, la perde, e al `Ready` il riduttore rimanda lo stato.
#[derive(Debug)]
pub enum MediaOp {
    /// Il brano mostrato, o niente a player fermo.
    Show(Option<Box<Now>>),
    Volume(f32),
}

/// Un'operazione su Discord Rich Presence.
#[derive(Debug)]
// `testing` re-exports the type, so its fields count as read there.
#[cfg_attr(
    all(target_os = "android", not(any(test, feature = "test-support"))),
    expect(dead_code, reason = "su Android non c'è Discord: l'host non legge i campi")
)]
pub enum DiscordOp {
    /// Avvia il thread per l'app Discord con questo identificativo.
    Start(String),
    /// Ferma il thread: l'attività sparisce da Discord.
    Stop,
    /// L'attività da mostrare, o nessuna (in pausa sparisce, come nelle altre app di musica).
    Set(Option<Box<DiscordNow>>),
}

/// Il brano che Discord mostra, con la posizione e la durata: l'host ne fa l'attività con l'ora del
/// sistema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordNow {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub cover_url: Option<String>,
    pub position: Duration,
    pub duration: Duration,
}

/// Ciò che si è mostrato l'ultima volta: se non cambia, non si manda niente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shown {
    track: Option<Uuid>,
    playing: bool,
    /// Un salto nel brano o un brano che riparte spostano la posizione che il sistema mostra.
    seeks: u64,
    started: u64,
    /// La copertina in pixel è arrivata.
    art: bool,
}

#[derive(Debug, Default)]
pub(crate) struct System {
    /// Il collegamento multimediale ha dato il suo capo: da qui in poi gli effetti arrivano.
    ready: bool,
    shown: Option<Shown>,
    volume: Option<f32>,
    art: Option<Artwork>,
    /// L'identificativo dell'app Discord con il thread acceso, se è acceso.
    discord: Option<String>,
    discord_shown: Option<Shown>,
}

impl System {
    /// Il collegamento è pronto: si rimanda tutto ciò che il sistema non ha ancora visto.
    pub(crate) fn ready(&mut self) {
        self.ready = true;
        self.shown = None;
        self.volume = None;
    }

    pub(crate) fn set_art(&mut self, art: Artwork) {
        self.art = Some(art);
    }

    /// L'impostazione di Discord è cambiata, o no: accende o spegne il thread. Senza Discord (Android)
    /// o con l'interruttore spento non parte niente.
    pub(crate) fn set_discord(&mut self, presence: bool, app_id: &str, effects: &mut Vec<Effect>) -> bool {
        let chosen = app_id.trim();
        let chosen = if chosen.is_empty() { DEFAULT_DISCORD_APP_ID } else { chosen };
        let wanted = (cfg!(not(target_os = "android")) && presence && !chosen.is_empty()).then_some(chosen);
        if self.discord.as_deref() == wanted {
            return false;
        }
        if self.discord.take().is_some() {
            effects.push(Effect::Discord(DiscordOp::Stop));
        }
        if let Some(id) = wanted {
            self.discord = Some(id.to_owned());
            effects.push(Effect::Discord(DiscordOp::Start(id.to_owned())));
        }
        self.discord_shown = None;
        true
    }

    /// Dopo ogni ingresso: se il brano, la pausa o la posizione sono cambiati per il sistema o per
    /// Discord, lo si dice.
    pub(crate) fn sync(&mut self, view: &PlaybackView, now: Instant, effects: &mut Vec<Effect>) {
        let wants_media = self.ready;
        if !wants_media && self.discord.is_none() {
            return;
        }
        let track = view.queue.current().filter(|_| view.is_active());
        let position = view.clock.at(now);
        let art =
            track.and_then(|t| self.art.as_ref().filter(|a| t.cover_url.as_deref() == Some(a.url.as_str())));
        let shown = Shown {
            track: track.map(|t| t.id),
            playing: view.is_playing(),
            seeks: view.seeks,
            started: view.started,
            art: art.is_some(),
        };
        if wants_media {
            if self.shown != Some(shown) {
                self.shown = Some(shown);
                let now = track.map(|track| {
                    Box::new(Now {
                        title: track.title.clone(),
                        artist: track.artist.name.clone(),
                        album: track.album.as_ref().map(|album| album.title.clone()),
                        cover_url: track.cover_url.clone(),
                        duration: track.duration(),
                        playing: shown.playing,
                        position,
                        artwork: art.map(|a| (a.side, a.rgba.clone())),
                    })
                });
                effects.push(Effect::Media(MediaOp::Show(now)));
            }
            if self.volume != Some(view.volume) {
                self.volume = Some(view.volume);
                effects.push(Effect::Media(MediaOp::Volume(view.volume)));
            }
        }
        if self.discord.is_some() {
            // In pausa o fermo l'attività è la stessa, qualunque cosa cambi: nessuna.
            let on_air = track.filter(|_| shown.playing);
            let key = on_air.map(|_| Shown { art: false, ..shown });
            if self.discord_shown != Some(key.unwrap_or(IDLE)) {
                self.discord_shown = Some(key.unwrap_or(IDLE));
                let activity = on_air.map(|track| {
                    Box::new(DiscordNow {
                        title: track.title.clone(),
                        artist: track.artist.name.clone(),
                        album: track.album.as_ref().map(|album| album.title.clone()),
                        cover_url: track.cover_url.clone(),
                        position,
                        duration: track.duration(),
                    })
                });
                effects.push(Effect::Discord(DiscordOp::Set(activity)));
            }
        }
    }
}

/// «Niente su Discord»: la chiave di quando non suona nulla.
const IDLE: Shown = Shown { track: None, playing: false, seeks: 0, started: 0, art: false };
