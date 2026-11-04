import AVFoundation

/// The app's `AVAudioSession`: playback category so audio keeps going with the
/// ring switch on and the app in the background, plus the interruptions and
/// route changes a music player has to answer.
///
/// The output itself is cpal's RemoteIO unit; it already reports route changes
/// and media-service resets as stream errors, which the engine answers by
/// reopening the device. What cpal cannot know is what the user meant, so the
/// pause on an unplugged headset and the resume after a phone call live here.
final class KopuzAudioSession {
    static let shared = KopuzAudioSession()

    private let lock = NSLock()
    private var configured = false
    private var active = false
    private var playing = false
    private var resumeAfterInterruption = false

    private init() {
        let center = NotificationCenter.default
        let session = AVAudioSession.sharedInstance()
        center.addObserver(
            self, selector: #selector(interrupted(_:)),
            name: AVAudioSession.interruptionNotification, object: session)
        center.addObserver(
            self, selector: #selector(routeChanged(_:)),
            name: AVAudioSession.routeChangeNotification, object: session)
        center.addObserver(
            self, selector: #selector(mediaServicesReset(_:)),
            name: AVAudioSession.mediaServicesWereResetNotification, object: session)
    }

    /// Sets the category once, before the first output opens: changing it
    /// under a running RemoteIO unit is itself a route change, which cpal
    /// reports as a lost device.
    func activate() {
        lock.lock()
        defer { lock.unlock() }
        let session = AVAudioSession.sharedInstance()
        do {
            if !configured {
                try session.setCategory(.playback, mode: .default, policy: .longFormAudio)
                configured = true
            }
            if !active {
                try session.setActive(true)
                active = true
            }
        } catch {
            NSLog("kopuz: audio session activation failed: \(error)")
        }
    }

    /// Lets other apps' audio resume once nothing is queued here.
    func deactivate() {
        lock.lock()
        defer { lock.unlock() }
        guard active else { return }
        do {
            try AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
            active = false
        } catch {
            NSLog("kopuz: audio session deactivation failed: \(error)")
        }
    }

    func setPlaying(_ playing: Bool) {
        lock.lock()
        self.playing = playing
        lock.unlock()
    }

    /// A call, an alarm or Siri took the audio. The system has already
    /// stopped the output; the queue is paused to match, and resumed afterwards
    /// only when it was playing and the system says resuming is appropriate.
    @objc private func interrupted(_ note: Notification) {
        guard let info = note.userInfo,
            let raw = info[AVAudioSessionInterruptionTypeKey] as? UInt,
            let type = AVAudioSession.InterruptionType(rawValue: raw)
        else { return }
        switch type {
        case .began:
            lock.lock()
            resumeAfterInterruption = playing
            active = false
            lock.unlock()
            Rust.send("pause")
        case .ended:
            let options = AVAudioSession.InterruptionOptions(
                rawValue: info[AVAudioSessionInterruptionOptionKey] as? UInt ?? 0)
            lock.lock()
            let resume = resumeAfterInterruption && options.contains(.shouldResume)
            resumeAfterInterruption = false
            lock.unlock()
            if resume {
                activate()
                Rust.send("play")
            }
        @unknown default:
            break
        }
    }

    /// Unplugging headphones must not move the music to the speaker.
    @objc private func routeChanged(_ note: Notification) {
        guard let raw = note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt,
            let reason = AVAudioSession.RouteChangeReason(rawValue: raw),
            reason == .oldDeviceUnavailable
        else { return }
        Rust.send("pause")
    }

    /// Every audio object is gone after a reset, the session's category with
    /// them; the next activation sets it up again.
    @objc private func mediaServicesReset(_ note: Notification) {
        lock.lock()
        configured = false
        active = false
        lock.unlock()
        activate()
    }
}
