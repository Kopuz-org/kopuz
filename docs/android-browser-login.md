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

Kopuz supports experimental OAuth authorization for YouTube Music, following
[ytmusicapi's registered-client setup](https://ytmusicapi.readthedocs.io/en/stable/setup/oauth.html).
In your Google Cloud project, enable the YouTube Data API, configure the consent
screen (including your account as a test user if the app is in testing), and create
an OAuth client of type **TVs and Limited Input devices**. Enter its Client ID and
Client Secret in Kopuz.

Tap sign in. Kopuz opens a temporary page in your selected browser with a device
code. Copy that code, open the Google authorization link, enter the code and grant
access, then return to Kopuz. No redirect URI is required. The temporary page uses
`http://127.0.0.1:8897`; Google authorization itself uses HTTPS.

Kopuz polls Google's token endpoint, checks that YouTube Music accepts the granted
session, and stores and refreshes the token for account requests. Authorization
is sent as a Bearer token, without reading or importing browser cookies.

Google documents this flow for [limited-input devices](https://developers.google.com/identity/protocols/oauth2/limited-input-device),
not as its recommended native Android sign-in integration. Its use here is
experimental: an actual Google grant and YouTube Music account have not been
verified on Android. A rejected grant/session reports an error and does not mark
the source connected. Anonymous browsing remains available.

## Credential storage and verification

App credentials and SoundCloud/YouTube Music rotating tokens stay in the daemon's database,
outside `settings.toml` and the settings API. Secret fields are write-only;
leaving one blank when editing keeps the stored value. Changing app credentials
requires signing in again. Removing a source deletes its app credentials.

The sign-in listeners bind only to `127.0.0.1`. Spotify, SoundCloud and Apple
Music time out after five minutes; YouTube Music follows Google's device-code
expiry, capped at thirty minutes. If the browser cannot connect after that,
return to Kopuz and start sign-in again.

Provider account sign-in needs your own registered credentials. Compilation and
local regression checks do not verify consent, subscription access, or playback
against a live provider account.
