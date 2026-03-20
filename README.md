# Rusic

Rusic is a modern, lightweight, music player application built with Rust and the Dioxus framework. It provides a clean and responsive interface for managing and enjoying your local music collection.

[![Discord](https://img.shields.io/badge/Discord-5865F2?style=flat&logo=discord&logoColor=white)](https://discord.gg/K6Bmzw2E4M)
![Rust](https://img.shields.io/badge/Rust-000000?style=flat&logo=rust&logoColor=white)

![Rusic](https://github.com/user-attachments/assets/15d4942d-e9a0-404f-ad38-f292d224eaf1)

## Overview

Rusic allows you to scan your local directories for audio files, or you jellyfin library, automatically organizing them into a browsable library. You can navigate by artists, albums, or explore your custom playlists. The application is built for performance and desktop integration, utilizing the power of Rust.

## Features

- **Theming**: Includes dynamic theming support to customize the visual appearance.
- **Native Integration**: Integrates with system media controls (MPRIS) and "Now Playing" displays.
- **Discord RPC**: Embedded RPC included!!!
- **Double Option**: Yes, you can also use your jellyfin server to listen to your music coming from your server!
- **Lyrics Support**: Enjoy real-time synced and plain lyrics, complete with auto-scrolling to follow along with your music.
- **High Performance**: Heavy background processing and an optimized library scanner ensure the app opens instantly, runs smoothly, and skips previously indexed files quickly.
- **Auto-Cleanup**: Automatically removes missing or deleted tracks from your library when rescanning.
- **Smooth Navigation**: Enjoy a polished interface where scroll positions reset properly as you browse different views and pages.

## Installation

### NixOS / Nix

**Run directly without installing:**

```bash
nix run github:temidaradev/rusic
```

**Install to your profile:**

```bash
nix profile add github:temidaradev/rusic
```

**NixOS flake (recommended — installs as a proper system app with icon & `.desktop` entry):**

Add rusic to your `flake.nix` inputs:

```nix
inputs.rusic.url = "github:temidaradev/rusic";
```

Pass it through to your system config and add the Cachix substituter so it downloads the pre-built binary instead of compiling:

```nix
# nixos/nix/default.nix
nix.settings = {
  substituters      = [ "https://cache.nixos.org" "https://rusic.cachix.org" ];
  trusted-public-keys = [
    "cache.nixos.org-1:6NCHdD59X431o0gWypbMrAURkbJ16ZPMQFGspcDShjY="
    "rusic.cachix.org-1:WXMpGpamblLUiJtcoxBxGGGGwIcWxGPJBUxarLiqWmw="
  ];
};
```

Then install the package:

```nix
# configuration.nix || machine.nix
environment.systemPackages = [
  rusic.packages.${system}.default
];
```


### Flatpak (Recommended)

Rusic is soon available on Flathub. To install from source manifest:

```bash
git clone https://github.com/temidaradev/rusic
cd rusic
flatpak-builder --user --install --force-clean build-dir com.temidaradev.rusic.json
flatpak run com.temidaradev.rusic
```

You can also click on the file and open it with an app provider, for example KDE discover

### Build from Source

```bash
git clone https://github.com/temidaradev/rusic
cd rusic
npm install
dx serve --package rusic
```

### macOS

**Quarantine note:** If you downloaded a `.dmg` instead, macOS may block it. Run once to clear the quarantine flag:

```bash
xattr -d com.apple.quarantine /Applications/Rusic.app
```


### Where does Rusic keep its files?

On **macOS** everything lives under your Library folders:
- `~/Library/Application Support/com.temidaradev.rusic/config.json` — your settings
- `~/Library/Caches/com.temidaradev.rusic/library.json` — the scanned library
- `~/Library/Caches/com.temidaradev.rusic/playlists.json` — your playlists
- `~/Library/Caches/com.temidaradev.rusic/covers/` — cached album art

On **Linux** it follows the XDG spec like you'd expect:
- `~/.config/rusic/config.json` — your settings
- `~/.cache/rusic/library.json` — the scanned library
- `~/.cache/rusic/playlists.json` — your playlists
- `~/.cache/rusic/covers/` — cached album art

If covers aren't showing or the library looks off, just delete the cache folder and hit rescan.

### Scrobbling functionality

Scrobbling functionality is only available through MusicBrainz (for now). To enable it, you need to provide a valid MusicBrainz token in the configuration file. The scrobbling also is only available for your local musics. It's highly recommended to use [jellyfin-plugin-listenbrainz](https://github.com/lyarenei/jellyfin-plugin-listenbrainz), because if you also use other music apps for your jellyfin server, you can scrobble your music from anywhere.

## Optimization

rusic is built to feel snappy even with large libraries. here's what we do under the hood:

**skip what's already indexed** — the scanner keeps a `HashSet` of every path it's already seen, so rescans only process new files. if you have 10k tracks and add 5 new ones, it won't re-read the other 9995. makes a huge difference on HDDs especially.

**parallel startup loading** — on launch, library, config, playlists, and favorites all load in parallel with `tokio::join!`. before this, everything loaded sequentially and you'd stare at a blank window for a bit. now it's near-instant.

**album art caching** — cover images get extracted once and saved to disk (`~/.cache/rusic/covers/` on linux, `~/Library/Caches/` on mac). we also cache the macOS now-playing artwork object in memory so it doesn't re-decode the image every time the progress bar updates.

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

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=temidaradev/rusic&type=date&legend=top-left)](https://www.star-history.com/#temidaradev/rusic&type=date&legend=top-left)
