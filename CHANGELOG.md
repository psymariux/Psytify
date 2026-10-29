# Changelog

All notable changes are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Desktop client for Arch Linux and Windows, and an Android app with a phone layout.
- Lossless playback (FLAC, WAV) and lossy formats through Symphonia, with ranged streaming and seeking.
- Home feed, search, artist and album pages, queue, shuffle, repeat, sleep timer and autoplay.
- Time-synced lyrics in a side panel and an immersive view; optional music video.
- Local playlists, likes, follows and saved albums.
- Downloads at a chosen quality when the license allows them.
- Jam: shared listening for up to 8 people.
- MPRIS media controls on Linux and an optional Clerk sign-in.
- Arch Linux package builder (`scripts/arch-package.sh`) and Android APK builder.
- Backend address configurable at run time (`PSYTIFY_API_URL`) and at build time (`PSYTIFY_DEFAULT_API_URL`).
- Dedicated phone layout: four tabs, a phone Home and Library, a thumb-sized full-screen player.
- Backend address editable in Settings → About, with a reset to the build default.
- Clerk sign-in configurable at build time (`PSYTIFY_DEFAULT_CLERK_ISSUER`, `PSYTIFY_DEFAULT_CLERK_CLIENT_ID`)
  and working on Android, where the sign-in page opens in the system browser.
- In-app sign-in branded as Psytify: email or username and password, sign-up with an email code,
  and GitHub, Google or Discord through the browser. Talks to the Clerk Frontend API directly,
  so the build needs only `PSYTIFY_DEFAULT_CLERK_ISSUER`.
- Profiles and friends: search people by username, send and accept friend requests, and open a
  friend's profile with what they are playing now, recent listens, top artists of the last 30 days,
  public playlists and mutual friends. Sharing listening activity can be turned off.
- Public playlists: any local playlist can be published to your profile.
- Android: playback controls on the lock screen and in the notification (title, artist, cover,
  Play, Pause, Next, Previous and seeking), also from headphones.
- Phone player: lyrics and the queue take the space under the cover, the progress bar can be dragged
  with a finger, and the cover shrinks to a thumbnail when lyrics or the queue are open.
- Discord Rich Presence on desktop ("Listening to Psytify" with track, artist, cover and time),
  through Discord's local IPC; build-time Application ID in `PSYTIFY_DEFAULT_DISCORD_APP_ID`.

### Changed

- `PSYTIFY_DEFAULT_CLERK_CLIENT_ID` and `clerk_client_id` are no longer used by the client.

[Unreleased]: https://github.com/psymariux/Psytify/commits/main
