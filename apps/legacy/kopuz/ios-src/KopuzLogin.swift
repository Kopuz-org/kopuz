import UIKit
import WebKit

/// The in-app browser for provider sign-ins (YouTube Music, SoundCloud, Apple
/// Music, Spotify), the counterpart of Android's `LoginActivity`.
///
/// Desktop harvests cookies from a spawned system browser's profile; an iOS app
/// has no such thing, so this sheet plays that role. Each sign-in gets a fresh,
/// non-persistent cookie jar, so a stale half-session can never satisfy the
/// Rust poller before the user has actually signed in. Swiping the sheet away
/// or tapping Cancel closes it, which the poller reports as a cancelled
/// sign-in.
final class KopuzLogin: NSObject, WKNavigationDelegate, WKHTTPCookieStoreObserver,
    UIAdaptivePresentationControllerDelegate
{
    static let shared = KopuzLogin()

    /// Google refuses account sign-in when the user agent reveals an embedded
    /// web view ("this browser or app may not be secure"), so the sheet
    /// presents itself as plain mobile Safari.
    private static let userAgent =
        "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 "
        + "(KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1"

    private let lock = NSLock()
    private var cookies: [HTTPCookie] = []
    private var open = false
    private var sheet: UINavigationController?
    private var webView: WKWebView?

    var isOpen: Bool {
        lock.lock()
        defer { lock.unlock() }
        return open
    }

    func open(_ url: URL) {
        onMain { self.present(url) }
    }

    func close() {
        onMain {
            self.sheet?.dismiss(animated: true)
            self.finish()
        }
    }

    /// The `Cookie:` header a request to `url` would carry.
    func cookieHeader(for url: URL) -> String? {
        guard let host = url.host?.lowercased() else { return nil }
        let path = url.path.isEmpty ? "/" : url.path
        lock.lock()
        let jar = cookies
        lock.unlock()
        let matching = jar.filter { cookie in
            let domain = cookie.domain.lowercased()
            let bare = domain.hasPrefix(".") ? String(domain.dropFirst()) : domain
            let hostMatches = host == bare || host.hasSuffix("." + bare)
            return hostMatches && path.hasPrefix(cookie.path)
                && (cookie.expiresDate.map { $0 > Date() } ?? true)
        }
        guard !matching.isEmpty else { return nil }
        return matching.map { "\($0.name)=\($0.value)" }.joined(separator: "; ")
    }

    private func present(_ url: URL) {
        if sheet != nil {
            webView?.load(URLRequest(url: url))
            return
        }
        guard let host = Self.topViewController() else {
            NSLog("kopuz: no view controller to present the sign-in sheet from")
            return
        }
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        let web = WKWebView(frame: .zero, configuration: configuration)
        web.customUserAgent = Self.userAgent
        web.navigationDelegate = self
        web.allowsBackForwardNavigationGestures = true
        configuration.websiteDataStore.httpCookieStore.add(self)

        let content = UIViewController()
        content.view = web
        content.navigationItem.leftBarButtonItem = UIBarButtonItem(
            systemItem: .cancel,
            primaryAction: UIAction { [weak self] _ in self?.close() })
        let sheet = UINavigationController(rootViewController: content)
        sheet.presentationController?.delegate = self

        lock.lock()
        cookies = []
        open = true
        lock.unlock()
        self.sheet = sheet
        webView = web
        host.present(sheet, animated: true)
        web.load(URLRequest(url: url))
    }

    /// Main thread only.
    private func finish() {
        webView?.configuration.websiteDataStore.httpCookieStore.remove(self)
        webView?.navigationDelegate = nil
        webView = nil
        sheet = nil
        lock.lock()
        open = false
        cookies = []
        lock.unlock()
    }

    private func refreshCookies(from store: WKHTTPCookieStore) {
        store.getAllCookies { [weak self] all in
            guard let self else { return }
            self.lock.lock()
            if self.open { self.cookies = all }
            self.lock.unlock()
        }
    }

    func cookiesDidChange(in cookieStore: WKHTTPCookieStore) {
        refreshCookies(from: cookieStore)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        refreshCookies(from: webView.configuration.websiteDataStore.httpCookieStore)
        sheet?.topViewController?.navigationItem.title = webView.url?.host
    }

    func presentationControllerDidDismiss(_ presentationController: UIPresentationController) {
        finish()
    }

    private static func topViewController() -> UIViewController? {
        let windows = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows)
        var top = (windows.first(where: \.isKeyWindow) ?? windows.first)?.rootViewController
        while let presented = top?.presentedViewController {
            top = presented
        }
        return top
    }
}
