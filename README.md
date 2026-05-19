<!--markdownlint-disable MD013 MD033 MD041 -->
<div align="center">
  <img src="crates/kopuz/assets/banner.png" alt="Kopuz Logo" height="300"/>
  <h1>Kopuz</h1>
  <p>
    Kopuz is a modern, lightweight, music player application built with Rust
    and the Dioxus framework. It provides a clean and responsive interface
    for managing and enjoying your local music collection.
  </p>
  <a href="https://discord.gg/K6Bmzw2E4M">
    <img src="https://img.shields.io/badge/Discord-5865F2?style=flat&logo=discord&logoColor=white" alt="Discord">
  </a>
  <img src="https://img.shields.io/badge/Rust-000000?style=flat&logo=rust&logoColor=white" alt="Rust">
  <img src="https://github.com/Kopuz-org/kopuz/actions/workflows/build.yml/badge.svg" alt="Build">
  <img src="https://github.com/user-attachments/assets/2b12ec40-2fcb-45e9-969e-ef99b4654957" alt="Kopuz">

</div>

## About the Name

The _kopuz_ is an ancient Turkic string instrument and is often considered the
ancestor of many Central Asian lutes. It was traditionally used by bards and
shamans.

The Kyrgyz _komuz_ is not the same instrument, but likely a descendant of the
_kopuz_. The Kazakh _kobyz_ is also related, though it is bowed rather than
plucked. In contrast, the Tuvan/Yakut _xomus_ (jaw harp) is unrelated, despite
the similar name.

In Turkic legend, the _kopuz_ is linked to Dede Korkut, a legendary bard, though
this is mythological rather than historical.

## Overview

Kopuz allows you to scan your local directories for audio files, or stream from
your Jellyfin or Subsonic (Navidrome, etc.) server, automatically organizing
everything into a browsable library. You can navigate by artists, albums,
genres, or explore your custom playlists. The application is built for
performance and desktop integration, utilizing the power of Rust.

## Features

[jellyfin-plugin-listenbrainz]: https://github.com/lyarenei/jellyfin-plugin-listenbrainz

- **Theming**: Includes dynamic theming support to customize the visual
  appearance. You can also build your own custom theme from scratch with full
  color variable control.
- **Native Integration**: Integrates with system media controls on Linux
  (MPRIS), macOS (Now Playing / Remote Command Center), and Windows (System
  Media Transport Controls).
- **Discord RPC**: Embedded RPC included!!!
- **Multiple Backends**: Stream from your Jellyfin or Subsonic-compatible server
  (Navidrome works great), or just point it at a local folder. Mix and match as
  you like.
- **Lyrics Support**: Enjoy real-time synced and plain lyrics, complete with
  auto-scrolling to follow along with your music.
- **Favorites**: Star tracks locally or sync favorites with your
  Jellyfin/Subsonic server.
- **Playlists**: Create and manage your own playlists, add individual tracks or
  whole albums at once, and sync playlists to your server.
- **Genre Browsing**: Browse your library by genre for both local and server
  music.
- **Search**: Search across artists, albums, and tracks with real-time results.
- **Listening Logs**: Tracks play counts locally so you can see what you
  actually listen to most.
- **Scrobbling**: Scrobble to ListenBrainz. For Jellyfin users,
  [jellyfin-plugin-listenbrainz] is recommended if you use multiple clients.
- **Language Support**: UI available in English, Russian, German, French,
  Spanish, Turkish, Ukrainian, Polish, Arabic, Greek, Hebrew, Hungarian,
  Indonesian, Japanese, Korean, Romanian, Brazilian Portuguese, Toki Pona, and
  Simplified Chinese with a streamlined experience for adding new languages.
- **High Performance**: Heavy background processing and an optimized library
  scanner ensure the app opens instantly, runs smoothly, and skips previously
  indexed files quickly.
- **Auto-Cleanup**: Automatically removes missing or deleted tracks from your
  library when rescanning.
- **Smooth Navigation**: Enjoy a polished interface where scroll positions reset
  properly as you browse different views and pages.
- **Reduce Animations**: Accessibility setting to tone down motion effects if
  you prefer a calmer UI.
- **Equalizer**: Built-in 5-band equalizer with presets and custom settings to
  fine-tune your sound.
- **Crossfade**: Blend track transitions for smoother automatic playback between
  songs on native desktop builds. Browser playback currently uses normal track
  switching.
- **Channel Mode**: Switch between `Stereo`, `Mono`, `Left only`, `Right only`,
  and `Swap L/R` output modes.
- **yt-dlp Integration**: Download audio directly from YouTube and other
  supported sites via yt-dlp. Choose your output format (Best Audio, MP3, FLAC,
  WAV, or MP4 video). FLAC is not recommended since yt-dlp remuxes lossy audio
  rather than decoding from a lossless source. Supports SponsorBlock, chapter
  splitting, cookies, rate limiting, and more. Requires `yt-dlp` installed on
  your system.
- **Metadata Settings**: A dedicated Metadata section in Settings lets you
  control how artist images are sourced. Choose between **Album Cover** (uses
  the first album artwork as the artist photo, default) or **Artist Photo**
  (fetches actual artist images directly from your Jellyfin or Subsonic server).
  When switching to Artist Photo mode, images are fetched from the server in the
  background as soon as you open the Artists page. If an artist has no dedicated
  photo on your server, their first album cover is used as a fallback so nothing
  ever shows blank.

## Installation

### NixOS / Nix

**Run directly without installing:**

```bash
nix run github:temidaradev/kopuz
```

**Install to your profile:**

```bash
nix profile add github:temidaradev/kopuz
```

**On NixOS, using the flake:**

> [!TIP]
> This is recommended over `nix profile` as it installs Kopuz as a proper system
> app with icon & `.desktop` entry.

Add Kopuz to your `flake.nix` inputs:

```nix
{
  inputs.kopuz.url = "github:temidaradev/kopuz";
}
```

Then pass it through to your system config and add the Cachix substituter so it
downloads the pre-built binary instead of compiling:

```nix
{
  nix.settings = {
    substituters      = ["https://kopuz.cachix.org" ];
    trusted-public-keys = ["kopuz.cachix.org-1:J2X3AnAYhKTJW5S3aCLoA1ckonQXVNZMQvhZA0YAufw="];
  };
}
```

Then install the package:

```nix
{pkgs, kopuz, ...}: let
  kopuzPkg = kopuz.packages.${pkgs.stdenv.hostPlatform.system}.default

in {
  environment.systemPackages = [kopuzPkg];
}
```

### AUR (Arch Linux)

Install from the AUR using your preferred helper:

```bash
yay -S kopuz
# or
paru -S kopuz
```

> **Note:** `dioxus-cli` must be installed first at the version matching dioxus
> 0.7.x:
>
> ```bash
> cargo install dioxus-cli --version "^0.7"
> ```

### Flatpak (Recommended)

Kopuz is soon available on Flathub. To install from source manifest:

```bash
git clone https://github.com/temidaradev/kopuz
cd kopuz
flatpak-builder --user --install --force-clean build-dir packaging/flatpak/com.temidaradev.kopuz.json
flatpak run com.temidaradev.kopuz
```

You can also click on the file and open it with an app provider, for example KDE
discover

### AppImage

> [!IMPORTANT]
> The AppImage requires `webkit2gtk-4.1` and `gtk3` installed on your system.
> Those dependencies are **not** bundled.
>
> On most distros with a modern desktop environment, these are already present.
> You will need to install them manually if they are not yet installed.

On Arch-based distros, if the AppImage crashes with a `WebKitNetworkProcess`
error, either run it with:

```bash
LD_LIBRARY_PATH=/usr/lib ./kopuz_*.AppImage
```

Or create symlinks once (requires sudo):

```bash
sudo mkdir -p /usr/libexec/webkit2gtk-4.1
sudo ln -s /usr/lib/webkit2gtk-4.1/WebKitNetworkProcess /usr/libexec/webkit2gtk-4.1/
sudo ln -s /usr/lib/webkit2gtk-4.1/WebKitWebProcess /usr/libexec/webkit2gtk-4.1/
sudo ln -s /usr/lib/webkit2gtk-4.1/WebKitGPUProcess /usr/libexec/webkit2gtk-4.1/
```

### Build from Source

#### Dependencies

**Using Nix**

> [!TIP]
> [Nix](https://nixos.org) is the primary means of development for Kopuz, and it
> is the recommended method for getting build dependencies in a pure,
> reproducible environment consistent across systems.

```bash
# Using Nix3 CLI
nix develop
```

If you are a [Direnv](https://direnv.net) user, use the provided `.envrc`:

```bash
# Using Direnv
direnv allow
```

Direnv is recommended if you want to keep using your usershell within the
development environment.

**Arch Linux Based Systems**

```bash
sudo pacman -S rust cargo dioxus-cli base-devel cmake pkgconf opus alsa-lib xdotool webkit2gtk-4.1 gtk3 libsoup3 openssl
```

**Debian Based Systems**

```bash
sudo apt install rustc cargo build-essential cmake pkg-config libopus-dev libasound2-dev libxdo-dev libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libssl-dev
cargo install dioxus-cli
```

**Fedora Based Systems**

```bash
sudo dnf groupinstall "Development Tools" "Development Libraries"
sudo dnf install rust cargo cmake pkgconf-pkg-config opus-devel alsa-lib-devel libxdo-devel webkit2gtk4.1-devel gtk3-devel libsoup3-devel openssl-devel
cargo install --locked dioxus-cli
```

**openSUSE Based Systems**

```bash
sudo zypper install rust cargo cmake pkg-config libopus-devel alsa-devel xdotool webkit2gtk3-soup2-devel gtk3-devel libsoup3-devel libopenssl-devel
cargo install --locked dioxus-cli
```

#### Developing Kopuz

```bash
# Clone the repository
$ git clone https://github.com/Kopuz-org/kopuz

# Move to the cloned directory
cd kopuz

# Install npm dependencies
npm install

# Serve project with Dioxus CLI
dx serve --package kopuz
```

### macOS

**Quarantine note:** If you downloaded a `.dmg` instead, macOS may block it. Run
once to clear the quarantine flag:

```bash
xattr -d com.apple.quarantine /Applications/Kopuz.app
```

### Where does Kopuz keep its files?

On **macOS** everything lives under your Library folders:

- `~/Library/Application Support/com.temidaradev.kopuz/config.json` - your
  settings
- `~/Library/Caches/com.temidaradev.kopuz/library.json` - the scanned library
- `~/Library/Caches/com.temidaradev.kopuz/playlists.json` - your playlists
- `~/Library/Caches/com.temidaradev.kopuz/covers/` - cached album art
- `~/Library/Caches/com.temidaradev.kopuz/offline_tracks/` - downloaded tracks

On **Linux** it follows the XDG spec like you'd expect:

- `~/.config/kopuz/config.json` - your settings
- `~/.cache/kopuz/library.json` - the scanned library
- `~/.cache/kopuz/playlists.json` - your playlists
- `~/.cache/kopuz/covers/` - cached album art
- `~/.cache/kopuz/offline_tracks/` - downloaded tracks

On **Windows** it uses your AppData folder:

- `%APPDATA%\temidaradev\kopuz\config\config.json` - your settings
- `%LOCALAPPDATA%\temidaradev\kopuz\cache\library.json` - the scanned library
- `%LOCALAPPDATA%\temidaradev\kopuz\cache\playlists.json` - your playlists
- `%LOCALAPPDATA%\temidaradev\kopuz\cache\covers\` - cached album art
- `%LOCALAPPDATA%\temidaradev\kopuz\cache\offline_tracks\` - downloaded tracks

If covers aren't showing or the library looks off, just delete the cache folder
and hit rescan.

## Optimization

Kopuz is built to feel snappy even with large libraries. Here's what we do under
the hood:

- **Skip what's already indexed** - the scanner keeps a `HashSet` of every path
  it's already seen, so rescans only process new files. If you have 10,000
  tracks, and then add 5 new ones, Kopuz will not re-read the other 9995. This
  makes a huge difference, especially on HDDs.

- **Parallel startup loading** - on launch, library, config, playlists, and
  favorites all load in parallel with `tokio::join!`. Before this change,
  everything loaded sequentially and you'd stare at a blank window for a bit.
  Now it's near-instant.

- **Album art caching** - cover images get extracted once and saved to disk
  (`~/.cache/kopuz/covers/` on Linux, `~/Library/Caches/` on macOS). We also
  cache the macOS now-playing artwork object in memory so it doesn't re-decode
  the image every time the progress bar updates.

- **Lazy loading images** - album covers in search results, track rows, and
  genre views all use `loading="lazy"` so we're not loading hundreds of images
  at once when you scroll through a big library.

- **Non-blocking I/O** - all the heavy stuff (metadata parsing, file scanning,
  saving library state) runs on `spawn_blocking` threads so the UI never
  freezes. The main thread stays responsive even during a full library scan.

- **Smarter sorting** - we use `sort_by_cached_key` instead of regular
  `sort_by_key` for library views, which avoids recalculating the sort key (like
  `.to_lowercase()`) on every comparison. Small thing perhaps, but it adds up
  with thousands of tracks.

- **HTTP caching for artwork** - the custom `artwork://` protocol serves images
  with `Cache-Control: public, max-age=31536000` so the Webview doesn't
  re-request covers it already has.

Overall, these changes brought the rescan time down _significantly_ and the app
feels much more responsive, especially with libraries over 5000 tracks. Memory
usage stays reasonable too since we're not holding decoded images in memory
longer than needed.

## Tech Stack

- **Dioxus**: UI Framework
- **Symphonia**: Audio decoding library
- **Cpal**: Audio I/O library
- **Lofty**: Metadata parsing
- **TailwindCSS**: Styling framework based on CSS

## Crypto Donation

- **Solana**: "BK84dVEMnGBP5Tya2mEaB1BQgcSBjngf1NBmRCqefxGg"
- **Bitcoin**: "bc1qz94yz9xvufa6hxlvjzaajgd2zyfu86arn68hu4"
- **Monero**:
  "86mz3HxTrKyYpuvx78m6pufbXdwAnoyoZBztz6HyYrnM1XP5YVrMy9jTVRY5vzgGtkizACLpFwHEdafKTMoj6y8mAVgvWMz"
- **Ethereum**: "0xa490D50470cdFf837B6663F7f6cBe50B157224e5"
- **USDT on Solana Chain**: "GYmnAcrA5MbF6cUxT2m5d5cwdfr14qSY9WFYRwXxaibW"

## Star History

[![Star History Chart](https://api.star-history.com/chart?repos=Kopuz-org/kopuz&type=date&legend=top-left)](https://www.star-history.com/?repos=Kopuz-org%2Fkopuz&type=date&legend=top-left)
