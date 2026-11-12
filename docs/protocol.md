# kopuz:// links

A `kopuz://` link opens a page, runs a search, starts playback or offers a
source to add in the Kopuz app. Use one anywhere a URL goes: a web page, a chat message, a launcher, a
script.

```html
<a href="kopuz://search?q=blue%20monday">Find it in Kopuz</a>
```

```sh
xdg-open 'kopuz://open/library'     # Linux
open 'kopuz://open/library'         # macOS
start kopuz://open/library          # Windows
```

If Kopuz is running, the link goes to its window, which comes to the front. If
it is not, the system starts Kopuz and the link opens once the app is up.

## Links

| Link | What it does |
| --- | --- |
| `kopuz://open/<page>` | Opens a page: `home`, `library`, `discover`, `radio`, `favorites`, `activity`, `settings` |
| `kopuz://open/settings/<section>` | Opens one settings section, for example `settings/appearance` |
| `kopuz://album/<id>` | Opens an album |
| `kopuz://artist/<key>` | Opens an artist |
| `kopuz://playlist/<id>` | Opens a playlist |
| `kopuz://search?q=<text>` | Searches the active source for `<text>` |
| `kopuz://play/track/<key>` | Replaces the queue with one track and plays it |
| `kopuz://play/album/<id>` | Plays an album |
| `kopuz://play/artist/<key>` | Plays an artist's tracks |
| `kopuz://play/playlist/<id>` | Plays a playlist |
| `kopuz://queue/track/<key>` | Adds one track to the end of the queue |
| `kopuz://add-source/<service>?...` | Opens the Add source form already filled in (see below) |

Links navigate, search and change the queue on their own. Nothing in the
grammar edits or deletes the library or changes a setting, and a source is
only added when the person presses Save on the form the link filled in. A link
from an untrusted page can do no more than start music or show that form.

## Adding a source

A provider can hand a user one link that sets up their account:

```
kopuz://add-source/subsonic?name=My%20Music&url=https%3A%2F%2Fmusic.example&username=alice
```

`<service>` is a service id from `kopuzctl services`, such as `subsonic` or
`jellyfin`. The query answers the service's own form:

| Parameter | What it fills in |
| --- | --- |
| `name` | The source's name |
| any field key | That field of the service's form; `kopuzctl services <service>` lists the keys, `url` for a server's address |
| `username` | The username in the sign-in form that follows, for services that sign in with one |
| `password` | The password in that sign-in form |

The link opens the Add source form with those answers in it. The user checks
them and saves; Kopuz then makes the source active and signs in. With both
`username` and `password` in the link the sign-in runs on its own, and its
form only stays up if the server refuses them. With one or neither, the form
opens with what the link gave, for the user to finish. Secret fields
and folder paths are never taken from a link, and a parameter the service has
no field for is ignored. An unknown service opens the form with an error.

A password in a link ends up wherever the link does: browser history, chat
logs, the clipboard. Send `password` only when it is an app password or a
one-time one that the user can revoke, never the account's own; otherwise send
`username` alone and let the user type it.

Clippsly, for example, signs in with a username and an App Password, so its
site can offer each user:

```
kopuz://add-source/clippsly?name=Clippsly&username=<user>&password=<app-password>
```

## Ids

Ids and keys are the ones the daemon hands out: `key` and `album_id` on a track
row, `id` on an album or a playlist, `key` on an artist credit. Get them from
[`kopuzctl`](cli.md) (`kopuzctl search "..."` prints track keys) or from the
[API](api.md).

An id belongs to the source it came from. A link carries no source, so it is
resolved against whichever source is active in the app, and an id that source
does not know opens an empty page or plays nothing. Links are therefore
dependable for a user's own library and scripts, and for `search` and `open`
on any install; a `play/track` link published for other people only works for
those whose active source knows that key.

## Encoding

Percent-encode every id, key and query value. Keys often contain `/` and `:`,
which must be `%2F` and `%3A` so they stay one path segment:

```
kopuz://play/track/local%3A%2Fmusic%2Fone.flac
```

In JavaScript, `encodeURIComponent(key)` does it. In the `q` value a `+` is
read as a space. The scheme and the first word (`open`, `play`, ...) are not
case-sensitive; ids and keys are.

A link the app cannot read shows "This link can't be opened" and does nothing
else.

## Listen on Kopuz links

`kopuz://j/<payload>` is the link behind the "Listen on Kopuz" button on a
Discord card. The daemon makes and reads the payload, which carries a song,
a position and a queue; it is not meant to be written by hand. The website
form, `https://kopuz.moe/j#<payload>`, opens it when Kopuz is installed.

## Kopuz Nightly

Kopuz and Kopuz Nightly both answer `kopuz://`. With both installed, the
system picks which one a link opens.

## Where the scheme is registered

| Install | Registered by |
| --- | --- |
| deb, rpm, AUR, Nix, Kopuz Nightly's Flatpak | the desktop entry (`x-scheme-handler/kopuz`) |
| AppImage | the desktop entry inside it, once a tool such as AppImageLauncher integrates it |
| macOS app | `CFBundleURLTypes` in the bundle |
| Windows installer (NSIS and MSI) | `Software\Classes\kopuz` |

A binary run from a checkout is not registered. Pass the link as an argument
instead, which is also how the system delivers it on Linux and Windows:

```sh
kopuz 'kopuz://open/radio'
```

To check the registration on Linux: `xdg-mime query default x-scheme-handler/kopuz`
prints `moe.kopuz.kopuz.desktop`.

Links are handled by the GPUI app (`kopuz`), not by the legacy app or `kopuzd`
on its own. To control a headless daemon, use `kopuzctl` or the API.
