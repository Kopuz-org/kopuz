# Rusic

Rusic is a modern, lightweight music player application built with Rust and the Dioxus framework. It provides a clean and responsive interface for managing and enjoying your local music collection.

![Rusic Interface](https://github.com/user-attachments/assets/8366b1ea-021f-4631-a97b-5ed6e5bf1562)

## Overview

Rusic allows you to scan your local directories for audio files, automatically organizing them into a browsable library. You can navigate by artists, albums, or explore your custom playlists. The application is built for performance and desktop integration, utilizing the power of Rust (well most of it gets demolished by webview .d).

## Features

- **Library Management**: Automatically scans your music folder to populate your library with artist and album metadata.
- **Playback Control**: Full suite of media controls including play, pause, skip, volume, and seeking.
- **Fullscreen Player**: An immersive mode that focuses on album artwork and playback details.
- **Theming**: Includes dynamic theming support to customize the visual appearance.
- **Native Integration**: Integrates with system media controls and "Now Playing" displays (only macOS for now).

## Installation

The easiest way to get Rusic running on Linux is the install script. It handles dependencies, builds the app, and adds it to your application menu automatically.

```bash
git clone https://github.com/temidaradev/rusic
cd rusic
chmod +x install.sh
./install.sh
```

**Note:** If `~/.local/bin` isn't in your PATH, you might need to add it to your `.zshrc` or `.bashrc`:
```bash
export PATH="$HOME/.local/bin:$PATH"
```

For mac you can just install the dmg file from releases

## Features

- **Blazing fast**: Built with Rust, so it doesn't eat your RAM like Electron players do.
- **Library Management**: Scans your folders and organizes music by artist and album automatically.
- **Integrated**: Works with your system media controls and "Now Playing" widgets. (only mac for now)
- **Clean UI**: Modern interface with Tailwind CSS and Dioxus.

## Development

If you want to hack on it:

```bash
# Install dependencies
npm install

# Run in dev mode
dx serve
```

## Tech Stack

- **Dioxus**: UI Framework
- **Rodio**: Audio playback
- **Lofty**: Metadata parsing
- **TailwindCSS**: All the styling
