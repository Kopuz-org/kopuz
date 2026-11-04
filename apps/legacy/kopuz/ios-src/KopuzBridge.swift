import Foundation

/// The C ABI the Rust side (`daemon/player/src/systemint/ios.rs`) links against.
/// Every export is safe to call from any thread: UIKit work hops to the main
/// queue here, never in Rust.

public typealias KopuzActionCallback = @convention(c) (UnsafePointer<CChar>?, Double) -> Void

/// Matches `RepeatMode::as_swift` in `ios.rs`.
enum KopuzRepeat: Int32 {
    case off = 0
    case playlist = 1
    case track = 2
}

/// The way back into Rust: the callback `kopuz_ios_init` was handed.
enum Rust {
    nonisolated(unsafe) static var onAction: KopuzActionCallback?

    static func send(_ action: String, _ value: Double = 0) {
        guard let callback = onAction else { return }
        action.withCString { callback($0, value) }
    }
}

func onMain(_ work: @escaping () -> Void) {
    if Thread.isMainThread {
        work()
    } else {
        DispatchQueue.main.async(execute: work)
    }
}

@_cdecl("kopuz_ios_init")
public func kopuzIosInit(_ onAction: KopuzActionCallback) {
    Rust.onAction = onAction
    KopuzAudioSession.shared.activate()
    onMain { KopuzNowPlaying.shared.registerCommands() }
}

@_cdecl("kopuz_ios_configure_webview")
public func kopuzIosConfigureWebView() {
    onMain { KopuzWebView.configure(attempt: 0) }
}

@_cdecl("kopuz_ios_now_playing")
public func kopuzIosNowPlaying(
    _ title: UnsafePointer<CChar>,
    _ artist: UnsafePointer<CChar>,
    _ album: UnsafePointer<CChar>,
    _ duration: Double,
    _ position: Double,
    _ playing: Bool,
    _ artwork: UnsafePointer<CChar>?
) {
    if playing {
        KopuzAudioSession.shared.activate()
    }
    KopuzAudioSession.shared.setPlaying(playing)
    KopuzNowPlaying.shared.update(
        title: String(cString: title),
        artist: String(cString: artist),
        album: String(cString: album),
        duration: duration,
        position: position,
        playing: playing,
        artwork: artwork.map { String(cString: $0) }
    )
}

@_cdecl("kopuz_ios_modes")
public func kopuzIosModes(_ shuffle: Bool, _ repeatMode: Int32) {
    let mode = KopuzRepeat(rawValue: repeatMode) ?? .off
    onMain { KopuzNowPlaying.shared.setModes(shuffle: shuffle, repeatMode: mode) }
}

@_cdecl("kopuz_ios_stop_session")
public func kopuzIosStopSession() {
    KopuzAudioSession.shared.setPlaying(false)
    KopuzNowPlaying.shared.clear()
    KopuzAudioSession.shared.deactivate()
}

@_cdecl("kopuz_ios_login_open")
public func kopuzIosLoginOpen(_ url: UnsafePointer<CChar>) -> Bool {
    guard let url = URL(string: String(cString: url)) else { return false }
    KopuzLogin.shared.open(url)
    return true
}

@_cdecl("kopuz_ios_login_cookies")
public func kopuzIosLoginCookies(_ url: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>? {
    guard let url = URL(string: String(cString: url)),
        let header = KopuzLogin.shared.cookieHeader(for: url)
    else { return nil }
    return strdup(header)
}

@_cdecl("kopuz_ios_login_is_open")
public func kopuzIosLoginIsOpen() -> Bool {
    KopuzLogin.shared.isOpen
}

@_cdecl("kopuz_ios_login_close")
public func kopuzIosLoginClose() {
    KopuzLogin.shared.close()
}

/// Frees a string this side `strdup`ed, so the allocator that made it
/// releases it.
@_cdecl("kopuz_ios_free")
public func kopuzIosFree(_ pointer: UnsafeMutablePointer<CChar>?) {
    free(pointer)
}
