import UIKit
import WebKit

/// The WKWebView wry puts on screen, set up for a page that draws edge to edge.
enum KopuzWebView {
    /// The first render can land before wry has attached the view, so a miss
    /// looks again shortly, a bounded number of times.
    static func configure(attempt: Int) {
        let webViews = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows)
            .flatMap { find(in: $0) }
        guard !webViews.isEmpty else {
            if attempt < 20 {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.1) {
                    configure(attempt: attempt + 1)
                }
            } else {
                NSLog("kopuz: no web view to configure")
            }
            return
        }
        for webView in webViews {
            webView.scrollView.contentInsetAdjustmentBehavior = .never
            webView.scrollView.bounces = false
            webView.isOpaque = false
            webView.backgroundColor = .black
        }
    }

    private static func find(in view: UIView) -> [WKWebView] {
        if let webView = view as? WKWebView {
            return [webView]
        }
        return view.subviews.flatMap { find(in: $0) }
    }
}
