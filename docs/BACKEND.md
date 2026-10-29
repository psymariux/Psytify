# Backend contract

Psytify is a client. This page describes what it expects from a backend, so that you can point it at
your own service. The client opens connections to **one** origin only (the `api_base_url`, see the
[README](../README.md#configuration)): audio, video, downloads and cover art are all fetched from that
origin, never from third-party hosts.

The server is not part of this repository.

## Conventions

- JSON, UTF-8. Dates are RFC 3339 in UTC; identifiers are UUID v4.
- Lists are paginated with `limit` (1–100, default 20) and `offset`, and answer
  `{"items": [], "limit": 20, "offset": 0, "next_offset": 20}`; `next_offset` is `null` on the last page.
- Errors have the shape `{"error": "not_found", "message": "…", "request_id": "…"}`. The client reads
  `error` and the HTTP status; `message` is for logs.
- `429` carries `Retry-After` (seconds). `503` means a dependency of the backend is down; the client
  shows a notice and retries on demand.
- The client sends no cookies. Sign-in is optional and adds `Authorization: Bearer <jwt>` to the
  requests that need it (`/v1/me…`, `/v1/users…`, `/v1/friends…`).

## Endpoints used by the client

| Endpoint | Purpose |
|---|---|
| `GET /v1/catalog/tracks`, `/v1/catalog/tracks/{id}` | Browse and read tracks (`q`, `sort=recent\|title`) |
| `GET /v1/catalog/search?q=` | Search tracks, albums and artists |
| `GET /v1/catalog/albums`, `/v1/catalog/albums/{id}`, `GET /v1/catalog/artists/{id}` | Albums and artists |
| `GET /v1/meta/search?q=`, `/v1/meta/artists?name=`, `/v1/meta/{artists,albums,playlists}/{id}` | Official-catalog metadata: search, artist pages with their full discography, albums, playlists |
| `POST /v1/meta/artists/{id}/open`, `POST /v1/meta/tracks/{id}/resolve` | Prepare an artist page; turn an official-catalog track into a playable one |
| `GET /v1/discover?q=`, `GET /v1/libraries/{identifier}`, `POST /v1/discover/{identifier}/import` | Open-licensed music from a public library |
| `GET /v1/sources?q=`, `/v1/sources/playlists…`, `POST /v1/sources/{source}/{id}/import` | Search and import from external sources |
| `GET /v1/tracks/{id}/stream-url` | Signed link to play a track (`quality=lossless\|standard`) |
| `GET /v1/tracks/{id}/download-url` | Signed link to save a track (`quality=low\|medium\|high\|lossless`); `403 not_downloadable` when the license does not allow it; `202` with `retry_after_secs` while a version is prepared |
| `GET /v1/tracks/{id}/video-url`, `POST /v1/tracks/{id}/video/prepare` | Optional music video |
| `GET /v1/tracks/{id}/lyrics` | Lyrics: `synced` (LRC) and/or `plain`; `found: false` is a `200` |
| `POST /v1/jam`, `GET /v1/jam/{code}/ws?name=` | Shared listening room (WebSocket) |
| `GET /v1/me` | Optional: who is signed in, with their profile (or `null`) |
| `PUT /v1/me/profile` | Create or update your profile: `handle` (3–30 of `a-z0-9_`, unique), `display_name`, `avatar_url`, `share_activity` |
| `PUT /v1/me/presence`, `DELETE /v1/me/presence` | What you are playing now (`track_id`, `position_ms`, `paused`); expires on its own |
| `POST /v1/me/listens` | A track you listened to (feeds recent listens and top artists) |
| `PUT /v1/me/playlists/{id}`, `DELETE /v1/me/playlists/{id}` | Publish or withdraw a public playlist (`name`, up to 500 `track_ids`) |
| `GET /v1/users?q=` | Find people by username or name |
| `GET /v1/users/{handle}`, `GET /v1/users/{handle}/playlists/{id}` | A profile page and one of its public playlists |
| `GET /v1/friends`, `POST /v1/friends/{handle}`, `DELETE /v1/friends/{handle}` | Friends with what they are playing, incoming and outgoing requests; send or accept, decline or remove |
| `GET /v1/media?t=…`, `GET /v1/cover?url=…` | Media and cover bytes (see below) |

## Track

```json
{
  "id": "6f1d2c3b-4a5e-4f60-8a7b-9c0d1e2f3a4b",
  "title": "Low Tide",
  "track_no": 1,
  "duration_ms": 215000,
  "audio_format": "flac",
  "downloadable": true,
  "has_video": false,
  "video_ready": false,
  "artist": {"id": "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d", "name": "Marea"},
  "album": {"id": "2b3c4d5e-6f7a-4b8c-9d0e-1f2a3b4c5d6e", "title": "Banchi di sabbia"},
  "licence": {"kind": "cc-by", "attribution": "Marea"},
  "cover_url": "https://example.org/cover.jpg",
  "created_at": "2026-10-01T09:30:00Z"
}
```

- `album` and `track_no` may be `null`; `cover_url` may be `null` (the client draws a cover itself).
- `audio_format`: `flac`, `mp3`, `ogg`, `wav`, `m4a`.
- `licence.kind` is one of `owned`, `cc0`, `cc-by`, `cc-by-sa`, `cc-by-nc`, `cc-by-nc-sa`, `cc-by-nd`,
  `cc-by-nc-nd`, `public-domain`, `live-trade`, `licensed-agreement`, `external`. The client shows the
  attribution for every `cc-by*` license.
- `downloadable` is authoritative: the client never offers a download when it is `false`.

## Signed links

`stream-url`, `download-url` and `video-url` answer:

```json
{"kind": "stream", "url": "/v1/media?t=…", "expires_at": "2026-10-01T09:35:00Z"}
```

- `url` is relative to the backend origin (an absolute URL is accepted too). The client joins it with
  `api_base_url`, and never logs it or writes it to disk.
- The target must support `Range` requests: playback starts before the file is complete, and seeking
  re-requests the byte range.
- When the target answers `401` or `403` the link has expired: the client asks for a new one and
  retries **once**.
- `download` links also carry `format` and `quality`, which say what is actually delivered.
- `video` links also carry `height`.

## Media and covers

- `GET /v1/media?t=<token>` returns `200`, `206` or `416` with `Content-Type`, `Content-Length`,
  `Content-Range` and `Accept-Ranges`. An invalid, tampered or expired token is `403`.
- `GET /v1/cover?url=<cover_url>` returns the image bytes (a size limit and a host allowlist are the
  backend's responsibility).

Serving everything through the backend keeps the client's network footprint to a single host and lets
the operator apply caching, authorization and rights checks in one place.

## Jam (shared listening)

A room holds up to 8 people who listen to the same queue in sync. The server only relays track
identifiers, position and members; each client resolves its own `stream-url`.

Client → server (JSON over the WebSocket):

```json
{"type": "state", "queue": ["<track id>"], "index": 0, "position_ms": 73500, "playing": true}
```

Server → client: `welcome` (your id, the code, members, last state or `null`), `members`, `state`
(the same fields plus `from` and `at_ms`) and `error` (`bad_message`, `invalid_state`, `not_in_room`,
`rate_limited`). At most 500 queued tracks and 10 messages per second per connection.

## Sign-in

Optional. The client has its own sign-in form and talks to the identity provider (Clerk) as a native
app: password, email code, or GitHub, Google and Discord through the system browser with a loopback
redirect. It sends the short-lived session token as a bearer. The backend validates it (signature,
issuer, expiry, and that it is a native session rather than a web one). Browsing and playback do not
require sign-in.

## Profiles and friends

A profile page always shows username, name, avatar, friend count, public playlists and mutual
friends. What someone is playing now, their recent listens and top artists of the last 30 days are
visible only to their friends, and only while they share activity (`share_activity`). `relation`
tells the client which button to show: `self`, `friend`, `incoming`, `outgoing` or `none`.

## Health

`GET /health` (liveness) and `GET /ready` (dependencies) are conventional for the operator and are not
called by the client.

## Rights

Every track served must have a documented source and license. The backend decides whether a track is
downloadable; the client only reflects that decision.
