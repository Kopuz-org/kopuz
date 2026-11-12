import MediaPlayer
import UIKit

/// The lock screen, Control Center, Dynamic Island and CarPlay card, and the
/// remote commands they send back.
final class KopuzNowPlaying {
    static let shared = KopuzNowPlaying()

    private let queue = DispatchQueue(label: "moe.kopuz.nowplaying")
    private var registered = false
    private var info: [String: Any] = [:]
    private var artworkRef: String?
    private var artwork: MPMediaItemArtwork?

    /// Main thread only: the command center hands its targets to UIKit.
    func registerCommands() {
        guard !registered else { return }
        registered = true
        let center = MPRemoteCommandCenter.shared()
        center.playCommand.addTarget { _ in
            Rust.send("play")
            return .success
        }
        center.pauseCommand.addTarget { _ in
            Rust.send("pause")
            return .success
        }
        center.togglePlayPauseCommand.addTarget { _ in
            Rust.send("toggle")
            return .success
        }
        center.nextTrackCommand.addTarget { _ in
            Rust.send("next")
            return .success
        }
        center.previousTrackCommand.addTarget { _ in
            Rust.send("prev")
            return .success
        }
        center.changePlaybackPositionCommand.addTarget { event in
            guard let event = event as? MPChangePlaybackPositionCommandEvent else {
                return .commandFailed
            }
            Rust.send("seek", event.positionTime)
            return .success
        }
        center.changeShuffleModeCommand.addTarget { event in
            guard let event = event as? MPChangeShuffleModeCommandEvent else {
                return .commandFailed
            }
            Rust.send("shuffle", event.shuffleType == .off ? 0 : 1)
            return .success
        }
        center.changeRepeatModeCommand.addTarget { event in
            guard let event = event as? MPChangeRepeatModeCommandEvent else {
                return .commandFailed
            }
            let mode: KopuzRepeat
            switch event.repeatType {
            case .one: mode = .track
            case .all: mode = .playlist
            default: mode = .off
            }
            Rust.send("repeat", Double(mode.rawValue))
            return .success
        }
        UIApplication.shared.beginReceivingRemoteControlEvents()
    }

    func setModes(shuffle: Bool, repeatMode: KopuzRepeat) {
        let center = MPRemoteCommandCenter.shared()
        center.changeShuffleModeCommand.currentShuffleType = shuffle ? .items : .off
        switch repeatMode {
        case .off: center.changeRepeatModeCommand.currentRepeatType = .off
        case .playlist: center.changeRepeatModeCommand.currentRepeatType = .all
        case .track: center.changeRepeatModeCommand.currentRepeatType = .one
        }
    }

    /// Called on every position tick, so the artwork is decoded once per
    /// picture and reused; a remote one is fetched in the background and lands
    /// on the card when it arrives, if the track has not moved on.
    func update(
        title: String, artist: String, album: String,
        duration: Double, position: Double, playing: Bool,
        artwork ref: String?
    ) {
        queue.async {
            var info: [String: Any] = [
                MPMediaItemPropertyTitle: title,
                MPMediaItemPropertyArtist: artist,
                MPMediaItemPropertyAlbumTitle: album,
                MPMediaItemPropertyPlaybackDuration: duration,
                MPNowPlayingInfoPropertyElapsedPlaybackTime: position,
                MPNowPlayingInfoPropertyPlaybackRate: playing ? 1.0 : 0.0,
                MPNowPlayingInfoPropertyDefaultPlaybackRate: 1.0,
                MPNowPlayingInfoPropertyMediaType: MPNowPlayingInfoMediaType.audio.rawValue,
            ]
            if ref != self.artworkRef {
                self.artworkRef = ref
                self.artwork = nil
                if let ref { self.loadArtwork(ref) }
            }
            if let artwork = self.artwork {
                info[MPMediaItemPropertyArtwork] = artwork
            }
            self.info = info
            self.publish(info)
        }
    }

    func clear() {
        queue.async {
            self.info = [:]
            self.artworkRef = nil
            self.artwork = nil
            DispatchQueue.main.async {
                MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
            }
        }
    }

    private func publish(_ info: [String: Any]) {
        DispatchQueue.main.async {
            MPNowPlayingInfoCenter.default().nowPlayingInfo = info
        }
    }

    /// Runs on `queue`.
    private func loadArtwork(_ ref: String) {
        if ref.hasPrefix("http://") || ref.hasPrefix("https://") {
            guard let url = URL(string: ref) else { return }
            URLSession.shared.dataTask(with: url) { data, _, _ in
                guard let data, let image = UIImage(data: data) else { return }
                let artwork = Self.artwork(of: image)
                self.queue.async {
                    guard self.artworkRef == ref, !self.info.isEmpty else { return }
                    self.artwork = artwork
                    self.info[MPMediaItemPropertyArtwork] = artwork
                    self.publish(self.info)
                }
            }.resume()
            return
        }
        let path = ref.hasPrefix("file://") ? String(ref.dropFirst("file://".count)) : ref
        if let image = UIImage(contentsOfFile: path) {
            artwork = Self.artwork(of: image)
        }
    }

    /// Kept off any actor: MediaPlayer calls the request handler on a queue of
    /// its own, and a handler inheriting main-actor isolation traps there.
    private static func artwork(of image: UIImage) -> MPMediaItemArtwork {
        MPMediaItemArtwork(boundsSize: image.size) { _ in image }
    }
}
