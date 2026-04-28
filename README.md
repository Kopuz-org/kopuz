# Kopuz (formerly known as Rusic)

Kopuz is a modern, lightweight, music player application built with Rust and the Dioxus framework. It provides a clean and responsive interface for managing and enjoying your local music collection.

## About the Name

The kopuz is an ancient Turkic string instrument and is often considered the ancestor of many Central Asian lutes. It was traditionally used by bards and shamans.

The Kyrgyz komuz is not the same instrument, but likely a descendant of the kopuz. The Kazakh kobyz is also related, though it is bowed rather than plucked. In contrast, the Tuvan/Yakut xomus (jaw harp) is unrelated, despite the similar name.

In Turkic legend, the kopuz is linked to Dede Korkut, a legendary bard, though this is mythological rather than historical.

[![Discord](https://img.shields.io/badge/Discord-5865F2?style=flat&logo=discord&logoColor=white)](https://discord.gg/K6Bmzw2E4M)
![Rust](https://img.shields.io/badge/Rust-000000?style=flat&logo=rust&logoColor=white)

![Kopuz](https://github.com/user-attachments/assets/554f7d71-5cce-4fe8-80c4-b72a2ce02fe6)

## Overview

Kopuz allows you to scan your local directories for audio files, or stream from your Jellyfin or Subsonic (Navidrome, etc.) server, automatically organizing everything into a browsable library. You can navigate by artists, albums, genres, or explore your custom playlists. The application is built for performance and desktop integration, utilizing the power of Rust.

## Features

- **Theming**: Includes dynamic theming support to customize the visual appearance. you can also build your own custom theme from scratch with full color variable control.
- **Native Integration**: Integrates with system media controls on Linux (MPRIS), macOS (Now Playing / Remote Command Center), and Windows (System Media Transport Controls).
- **Discord RPC**: Embedded RPC included!!!
- **Multiple Backends**: Stream from your Jellyfin or Subsonic-compatible server (Navidrome works great), or just point it at a local folder. mix and match as you like.
- **Lyrics Support**: Enjoy real-time synced and plain lyrics, complete with auto-scrolling to follow along with your music.
- **Favorites**: Star tracks locally or sync favorites with your Jellyfin/Subsonic server.
- **Playlists**: Create and manage your own playlists, add individual tracks or whole albums at once, and sync playlists to your server.
- **Genre Browsing**: Browse your library by genre for both local and server music.
- **Search**: Search across artists, albums, and tracks with real-time results.
- **Listening Logs**: Tracks play counts locally so you can see what you actually listen to most.
- **Scrobbling**: Scrobble to ListenBrainz. for Jellyfin users, [jellyfin-plugin-listenbrainz](https://github.com/lyarenei/jellyfin-plugin-listenbrainz) is recommended if you use multiple clients.
- **Language Support**: UI available in English and Russian, with more languages easy to add.
- **High Performance**: Heavy background processing and an optimized library scanner ensure the app opens instantly, runs smoothly, and skips previously indexed files quickly.
- **Auto-Cleanup**: Automatically removes missing or deleted tracks from your library when rescanning.
- **Smooth Navigation**: Enjoy a polished interface where scroll positions reset properly as you browse different views and pages.
- **Reduce Animations**: Accessibility setting to tone down motion effects if you prefer a calmer UI.

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

**NixOS flake (recommended — installs as a proper system app with icon & `.desktop` entry):**

Add kopuz to your `flake.nix` inputs:

```nix
inputs.kopuz.url = "github:temidaradev/kopuz";
```

Pass it through to your system config and add the Cachix substituter so it downloads the pre-built binary instead of compiling:

```nix
# nixos/nix/default.nix
nix.settings = {
  substituters      = [ "https://cache.nixos.org" "https://kopuz.cachix.org" ];
  trusted-public-keys = [
    "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
    "kopuz.cachix.org-1:WXMpGpamblLUiJtcoxBxGGGGwIcWxGPJBUxarLiqWmw="
  ];
};
```

Then install the package:

```nix
# configuration.nix || machine.nix
environment.systemPackages = [
  kopuz.packages.${system}.default
];
```


### Flatpak (Recommended)

Kopuz is soon available on Flathub. To install from source manifest:

```bash
git clone https://github.com/temidaradev/kopuz
cd kopuz
flatpak-builder --user --install --force-clean build-dir com.temidaradev.kopuz.json
flatpak run com.temidaradev.kopuz
```

You can also click on the file and open it with an app provider, for example KDE discover

### Build from Source

#### Dependencies
**Arch Linux Based Systems**
```bash
sudo pacman -S rust cargo dioxus-cli base-devel pkgconf opus alsa-lib xdotool webkit2gtk-4.1 gtk3 libsoup3 openssl
```
**Debian Based Systems**
```bash
sudo apt install rustc cargo build-essential pkg-config libopus-dev libasound2-dev libxdo-dev libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libssl-dev
cargo install dioxus-cli
```
**Fedora Based Systems**
```bash
sudo dnf groupinstall "Development Tools" "Development Libraries"
sudo dnf install rust cargo pkgconf-pkg-config opus-devel alsa-lib-devel libxdo-devel webkit2gtk4.1-devel gtk3-devel libsoup3-devel openssl-devel
cargo install dioxus-cli
```


```bash
git clone https://github.com/temidaradev/kopuz
cd kopuz
npm install
dx serve --package kopuz
```

### macOS

**Quarantine note:** If you downloaded a `.dmg` instead, macOS may block it. Run once to clear the quarantine flag:

```bash
xattr -d com.apple.quarantine /Applications/Kopuz.app
```


### Where does Kopuz keep its files?

On **macOS** everything lives under your Library folders:
- `~/Library/Application Support/com.temidaradev.kopuz/config.json` — your settings
- `~/Library/Caches/com.temidaradev.kopuz/library.json` — the scanned library
- `~/Library/Caches/com.temidaradev.kopuz/playlists.json` — your playlists
- `~/Library/Caches/com.temidaradev.kopuz/covers/` — cached album art

On **Linux** it follows the XDG spec like you'd expect:
- `~/.config/kopuz/config.json` — your settings
- `~/.cache/kopuz/library.json` — the scanned library
- `~/.cache/kopuz/playlists.json` — your playlists
- `~/.cache/kopuz/covers/` — cached album art

If covers aren't showing or the library looks off, just delete the cache folder and hit rescan.

## Optimization

kopuz is built to feel snappy even with large libraries. here's what we do under the hood:

**skip what's already indexed** — the scanner keeps a `HashSet` of every path it's already seen, so rescans only process new files. if you have 10k tracks and add 5 new ones, it won't re-read the other 9995. makes a huge difference on HDDs especially.

**parallel startup loading** — on launch, library, config, playlists, and favorites all load in parallel with `tokio::join!`. before this, everything loaded sequentially and you'd stare at a blank window for a bit. now it's near-instant.

**album art caching** — cover images get extracted once and saved to disk (`~/.cache/kopuz/covers/` on linux, `~/Library/Caches/` on mac). we also cache the macOS now-playing artwork object in memory so it doesn't re-decode the image every time the progress bar updates.

**lazy loading images** — album covers in search results, track rows, and genre views all use `loading="lazy"` so we're not loading hundreds of images at once when you scroll through a big library.

**non-blocking I/O** — all the heavy stuff (metadata parsing, file scanning, saving library state) runs on `spawn_blocking` threads so the UI never freezes. the main thread stays responsive even during a full library scan.

**smarter sorting** — we use `sort_by_cached_key` instead of regular `sort_by_key` for library views, which avoids recalculating the sort key (like `.to_lowercase()`) on every comparison. small thing but it adds up with thousands of tracks.

**http caching for artwork** — the custom `artwork://` protocol serves images with `Cache-Control: public, max-age=31536000` so the webview doesn't re-request covers it already has.

overall these changes brought the rescan time down significantly and the app feels much more responsive, especially with libraries over 5000 tracks. memory usage stays reasonable too since we're not holding decoded images in memory longer than needed.

## Tech Stack

- **Dioxus**: UI Framework
- **Rodio**: Audio playback library
- **Lofty**: Metadata parsing
- **TailwindCSS**: Styling framework based on CSS

## Crypto Donation

- **Solana**: "BK84dVEMnGBP5Tya2mEaB1BQgcSBjngf1NBmRCqefxGg"
- **Bitcoin**: "bc1qz94yz9xvufa6hxlvjzaajgd2zyfu86arn68hu4"
- **Monero**: "86mz3HxTrKyYpuvx78m6pufbXdwAnoyoZBztz6HyYrnM1XP5YVrMy9jTVRY5vzgGtkizACLpFwHEdafKTMoj6y8mAVgvWMz"
- **Ethereum**: "0xa490D50470cdFf837B6663F7f6cBe50B157224e5"
- **USDT on Solana Chain**: "GYmnAcrA5MbF6cUxT2m5d5cwdfr14qSY9WFYRwXxaibW"

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=temidaradev/kopuz&type=date&legend=top-left)](https://www.star-history.com/#temidaradev/kopuz&type=date&legend=top-left)
