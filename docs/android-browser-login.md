# Android browser sign-in

Kopuz opens the browser selected in each source's settings. **System default**
uses Android's default browser. An explicitly selected browser must be installed;
Kopuz reports an error instead of silently switching to another browser.

## Spotify

Register your own Spotify application and enter its Client ID when adding the
source. Register this exact redirect URI:

```text
http://127.0.0.1:8898/callback
```

Authorization uses PKCE. See Spotify's [redirect URI requirements](https://developer.spotify.com/documentation/web-api/concepts/redirect_uri).

## SoundCloud

Register your own application in the [SoundCloud developer portal](https://developers.soundcloud.com/docs/api/register-app).
Enter its Client ID and Client Secret in Kopuz and register this exact redirect:

```text
http://127.0.0.1:8899/callback
```

Kopuz uses SoundCloud's [OAuth authorization-code flow with PKCE](https://developers.soundcloud.com/docs),
then the public API for profile validation, search, liked tracks, playlists and
available AAC streams. Refresh tokens are single-use; Kopuz serializes refresh
and saves each replacement before using it again. API and subscription restrictions
still apply to individual tracks.

These are credentials for your own installation. Do not distribute an APK with
a shared Client Secret embedded in it. A broadly distributed confidential client
needs a service that keeps that secret on the server.

## Apple Music

Create a MusicKit identifier and key in your Apple Developer account and
[generate a developer token](https://developer.apple.com/documentation/applemusicapi/generating-developer-tokens).
Enter the signed developer token in Kopuz; keep the signing private key outside
the app. If your token has an `origin` restriction, include:

```text
http://127.0.0.1:8900
```

Kopuz serves a temporary sign-in page on that loopback address and opens it in
your selected browser. Tap **Continue with Apple Music**, then grant access in
Apple's MusicKit authorization window. The resulting Music User Token returns
to Kopuz through a state-checked, same-origin POST, never through a URL.

This integration connects your library using your developer token and the
Music User Token. **MusicKit playback is not integrated yet**; these accounts
cannot use the existing web-player playback or download path. Replace an expired
developer token in source settings and sign in again.

## YouTube Music

The current YouTube Music backend requires browser session cookies. Google's
[registered OAuth API](https://developers.google.com/youtube/v3/guides/auth/installed-apps)
authorizes the YouTube Data API and does not supply those cookies. Registered
Google OAuth is therefore not offered as a working login for this backend.
Anonymous mode remains available on Android.

## Credential storage and verification

App credentials and SoundCloud's rotating tokens stay in the daemon's database,
outside `settings.toml` and the settings API. Secret fields are write-only;
leaving one blank when editing keeps the stored value. Changing app credentials
requires signing in again. Removing a source deletes its app credentials.

The sign-in listeners bind only to `127.0.0.1` and time out after five minutes.
If the browser cannot connect after that, return to Kopuz and start sign-in again.

Provider account sign-in needs your own registered credentials. Compilation and
local regression checks do not verify consent, subscription access, or playback
against a live provider account.
