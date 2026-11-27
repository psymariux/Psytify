//! Profili, amici e attività (`docs/BACKEND.md`, «Profili e amici»): modelli e chiamate, tutte con il
//! token di sessione di Clerk.

use std::sync::Arc;

use reqwest::Method;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use super::client::ApiClient;
use super::models::{Page, Track};
use crate::error::AppError;

/// Il proprio profilo.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MyProfile {
    pub handle: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub share_activity: bool,
}

/// Il profilo di un altro, come lo vedono tutti.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Profile {
    pub handle: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    #[serde(rename = "self")]
    Me,
    Friend,
    /// L'altro ha chiesto l'amicizia.
    Incoming,
    /// La richiesta è partita da qui.
    Outgoing,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct NowPlaying {
    pub track: Track,
    pub position_ms: u64,
    pub paused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Listen {
    pub track: Track,
    pub listened_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TopArtist {
    pub id: Uuid,
    pub name: String,
    pub plays: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Activity {
    pub now_playing: Option<NowPlaying>,
    pub recent: Vec<Listen>,
    pub top_artists: Vec<TopArtist>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PlaylistSummary {
    pub id: Uuid,
    pub name: String,
    pub track_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UserPage {
    pub profile: Profile,
    pub relation: Relation,
    pub friend_count: u32,
    pub mutual_friends: Vec<Profile>,
    pub playlists: Vec<PlaylistSummary>,
    pub activity: Option<Activity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Friend {
    pub profile: Profile,
    pub now_playing: Option<NowPlaying>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct Friends {
    pub friends: Vec<Friend>,
    pub incoming: Vec<Profile>,
    pub outgoing: Vec<Profile>,
}

/// L'invito di un amico a una jam: il codice con cui entrare e chi l'ha mandato.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct JamInvite {
    pub code: String,
    pub from_handle: String,
    pub from_name: String,
}

#[derive(Deserialize)]
struct JamInvites {
    invites: Vec<JamInvite>,
}

#[derive(Deserialize)]
struct Invited {
    invited: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PublicPlaylist {
    pub id: Uuid,
    pub name: String,
    pub owner: Profile,
    /// Condivisi con la coda quando si preme play, senza copie.
    #[serde(deserialize_with = "shared_tracks")]
    pub tracks: Arc<[Track]>,
}

fn shared_tracks<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Arc<[Track]>, D::Error> {
    Vec::<Track>::deserialize(deserializer).map(Arc::from)
}

#[derive(Deserialize)]
struct RelationBody {
    relation: Relation,
}

/// Il profilo da creare o aggiornare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileDraft {
    pub handle: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub share_activity: Option<bool>,
}

/// Una risposta senza corpo dove ne serviva uno.
fn body<T>(reply: Option<T>) -> Result<T, AppError> {
    reply.ok_or(AppError::InvalidResponse)
}

impl ApiClient {
    pub async fn put_profile(&self, token: &str, draft: &ProfileDraft) -> Result<MyProfile, AppError> {
        let mut payload = json!({"handle": draft.handle, "display_name": draft.display_name, "avatar_url": draft.avatar_url});
        if let Some(share) = draft.share_activity {
            payload["share_activity"] = json!(share);
        }
        body(self.authed(Method::PUT, "/v1/me/profile", token, &[], Some(payload)).await?)
    }

    pub async fn search_users(&self, token: &str, q: &str) -> Result<Vec<Profile>, AppError> {
        let page: Page<Profile> =
            body(self.authed(Method::GET, "/v1/users", token, &[("q", q.to_owned())], None).await?)?;
        Ok(page.items)
    }

    pub async fn user_page(&self, token: &str, handle: &str) -> Result<UserPage, AppError> {
        body(
            self.authed(Method::GET, &format!("/v1/users/{}", path_segment(handle)), token, &[], None)
                .await?,
        )
    }

    pub async fn user_playlist(
        &self,
        token: &str,
        handle: &str,
        id: Uuid,
    ) -> Result<PublicPlaylist, AppError> {
        let path = format!("/v1/users/{}/playlists/{id}", path_segment(handle));
        body(self.authed(Method::GET, &path, token, &[], None).await?)
    }

    pub async fn friends(&self, token: &str) -> Result<Friends, AppError> {
        body(self.authed(Method::GET, "/v1/friends", token, &[], None).await?)
    }

    /// Chiede l'amicizia o accetta quella ricevuta.
    pub async fn add_friend(&self, token: &str, handle: &str) -> Result<Relation, AppError> {
        let path = format!("/v1/friends/{}", path_segment(handle));
        let reply: RelationBody = body(self.authed(Method::POST, &path, token, &[], None).await?)?;
        Ok(reply.relation)
    }

    /// Toglie l'amicizia, ritira o rifiuta la richiesta.
    pub async fn remove_friend(&self, token: &str, handle: &str) -> Result<(), AppError> {
        let path = format!("/v1/friends/{}", path_segment(handle));
        self.authed::<serde_json::Value>(Method::DELETE, &path, token, &[], None).await.map(|_| ())
    }

    /// Gli inviti alle jam ricevuti dagli amici e ancora validi.
    pub async fn jam_invites(&self, token: &str) -> Result<Vec<JamInvite>, AppError> {
        let reply: JamInvites = body(self.authed(Method::GET, "/v1/jam/invites", token, &[], None).await?)?;
        Ok(reply.invites)
    }

    /// Invita degli amici nella jam `code`; restituisce quanti inviti sono partiti.
    pub async fn invite_to_jam(
        &self,
        token: &str,
        code: &str,
        handles: &[String],
    ) -> Result<usize, AppError> {
        let path = format!("/v1/jam/{}/invite", path_segment(code));
        let payload = json!({"handles": handles});
        let reply: Invited = body(self.authed(Method::POST, &path, token, &[], Some(payload)).await?)?;
        Ok(reply.invited)
    }

    /// Toglie gli inviti alla jam `code`, rifiutati o già accettati.
    pub async fn dismiss_jam_invite(&self, token: &str, code: &str) -> Result<(), AppError> {
        let path = format!("/v1/jam/invites/{}", path_segment(code));
        self.authed::<serde_json::Value>(Method::DELETE, &path, token, &[], None).await.map(|_| ())
    }

    pub async fn put_presence(
        &self,
        token: &str,
        track: Uuid,
        position_ms: u64,
        paused: bool,
    ) -> Result<(), AppError> {
        let payload =
            json!({"track_id": track, "position_ms": position_ms.min(u64::from(u32::MAX)), "paused": paused});
        self.authed::<serde_json::Value>(Method::PUT, "/v1/me/presence", token, &[], Some(payload))
            .await
            .map(|_| ())
    }

    pub async fn delete_presence(&self, token: &str) -> Result<(), AppError> {
        self.authed::<serde_json::Value>(Method::DELETE, "/v1/me/presence", token, &[], None)
            .await
            .map(|_| ())
    }

    pub async fn post_listen(&self, token: &str, track: Uuid) -> Result<(), AppError> {
        let payload = json!({"track_id": track});
        self.authed::<serde_json::Value>(Method::POST, "/v1/me/listens", token, &[], Some(payload))
            .await
            .map(|_| ())
    }

    pub async fn put_playlist(
        &self,
        token: &str,
        id: Uuid,
        name: &str,
        tracks: &[Uuid],
    ) -> Result<(), AppError> {
        let payload = json!({"name": name, "track_ids": tracks});
        let path = format!("/v1/me/playlists/{id}");
        self.authed::<serde_json::Value>(Method::PUT, &path, token, &[], Some(payload)).await.map(|_| ())
    }

    pub async fn delete_playlist(&self, token: &str, id: Uuid) -> Result<(), AppError> {
        let path = format!("/v1/me/playlists/{id}");
        self.authed::<serde_json::Value>(Method::DELETE, &path, token, &[], None).await.map(|_| ())
    }
}

/// Un nome utente nel percorso: solo i caratteri che il server ammette, il resto non arriva.
fn path_segment(handle: &str) -> String {
    handle.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect()
}

/// Nome utente proposto per un profilo nuovo: quello di Clerk, oppure la parte dell'email prima
/// della chiocciola, ridotti alle regole del server (3-30 fra minuscole, cifre e `_`).
pub fn suggest_handle(username: Option<&str>, email: Option<&str>) -> String {
    let source = username.or_else(|| email.and_then(|e| e.split('@').next())).unwrap_or("ascoltatore");
    let mut handle: String = source
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .split('_')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    handle.truncate(24);
    while handle.len() < 3 {
        handle.push('_');
    }
    handle
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggested_handles_follow_the_server_rules() {
        assert_eq!(suggest_handle(Some("Psy.Mariux"), None), "psy_mariux");
        assert_eq!(suggest_handle(None, Some("ada.lovelace@example.com")), "ada_lovelace");
        assert_eq!(suggest_handle(None, None), "ascoltatore");
        assert_eq!(suggest_handle(Some("é"), None), "___");
        assert_eq!(suggest_handle(Some(&"x".repeat(40)), None).len(), 24);
    }

    #[test]
    fn jam_invites_read_the_documented_shape() {
        let reply: JamInvites = serde_json::from_value(json!({
            "invites": [{"code": "K7QXM3", "from_handle": "luca", "from_name": "Luca Rossi"}]
        }))
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(reply.invites[0].code, "K7QXM3");
        assert_eq!(reply.invites[0].from_name, "Luca Rossi");
    }

    #[test]
    fn handles_cannot_escape_the_path() {
        assert_eq!(path_segment("../admin?x=1"), "adminx1");
    }

    #[test]
    fn a_user_page_reads_the_documented_shape() {
        let page: UserPage = serde_json::from_value(json!({
            "profile": {"handle": "bea", "display_name": "Bea", "avatar_url": null},
            "relation": "self", "friend_count": 0, "mutual_friends": [], "playlists": [],
            "activity": {"now_playing": null, "recent": [], "top_artists": []}
        }))
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(page.relation, Relation::Me);
    }
}
