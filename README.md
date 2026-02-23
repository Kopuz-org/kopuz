# Rusic

Rusic is a modern, lightweight music player application built with Rust and the Dioxus framework. It provides a clean and responsive interface for managing and enjoying your local music collection.

![Rusic Interface](https://github.com/user-attachments/assets/5dd16aab-9b4e-44e6-afb5-60a36b08fd7c)

Join to the discord server created for Rusic!!

https://discord.gg/K6Bmzw2E4M

## Overview

Rusic allows you to scan your local directories for audio files, or you jellyfin library, automatically organizing them into a browsable library. You can navigate by artists, albums, or explore your custom playlists. The application is built for performance and desktop integration, utilizing the power of Rust.

## Features

- **Theming**: Includes dynamic theming support to customize the visual appearance.
- **Native Integration**: Integrates with system media controls (MPRIS) and "Now Playing" displays.
- **Discord RPC**: Embedded RPC included!!!
- **Double Option**: Yes you can also use your jellyfin server to listen to your music coming from your server!

## Installation

### Flatpak (Recommended)

Rusic is available on Flathub (coming soon). To install from source manifest:

```bash
git clone https://github.com/temidaradev/rusic
cd rusic
flatpak-builder --user --install --force-clean build-dir com.temidaradev.rusic.json
flatpak run com.temidaradev.rusic
```

or click on the file and open it with app provider like kde discover

### Build from Source


If you want to hack on it:

```bash
npm install

dx serve --package rusic
```

### MacOS Quarantine

Since Apple hates open-source software, they make it harder for users to install them since they don't explicitly "trust" them. The source code can be verified by yours truly though. But in the meantime, after downloading the .dmg and dragging the app to your ~/Applications, use:

```bash
xattr -d com.apple.quarantine /Applications/Rusic.app
```

## Tech Stack

- **Dioxus**: UI Framework
- **Rodio**: Audio playback
- **Lofty**: Metadata parsing
- **TailwindCSS**: All the styling

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=temidaradev/rusic&type=date&legend=top-left)](https://www.star-history.com/#temidaradev/rusic&type=date&legend=top-left)
