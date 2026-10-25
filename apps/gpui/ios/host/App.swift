import LocalAuthentication
import UIKit

// UIKit owns the window, the navigation bar and the safe areas. GPUI draws the content
// (`../../README.md`, "GPUI for every client"). Host pattern from longbridge/gpui-mobile
// example/ios/App.swift.
// The iOS 27 SDK refuses to launch an app without the UIScene life cycle, so a scene
// delegate owns the window (the upstream example's app-delegate window fails here).
@main
final class AppDelegate: UIResponder, UIApplicationDelegate {
    func application(_ application: UIApplication,
                     configurationForConnecting session: UISceneSession,
                     options: UIScene.ConnectionOptions) -> UISceneConfiguration {
        let config = UISceneConfiguration(name: nil, sessionRole: session.role)
        config.delegateClass = SceneDelegate.self
        return config
    }
}

final class SceneDelegate: UIResponder, UIWindowSceneDelegate {
    var window: UIWindow?

    func scene(_ scene: UIScene, willConnectTo session: UISceneSession,
               options: UIScene.ConnectionOptions) {
        guard let scene = scene as? UIWindowScene else { return }
        let window = UIWindow(windowScene: scene)
        window.rootViewController = UINavigationController(rootViewController: SjelController())
        window.makeKeyAndVisible()
        self.window = window
    }

    func sceneDidBecomeActive(_ scene: UIScene) {
        gpui_ios_did_become_active(nil)
        // The phone was in a pocket and the day moved on: the reminders it holds are the ones it
        // scheduled last time it was awake (`ios/src/notify.rs` rebuilds them).
        sjel_ios_became_active()
    }
    func sceneWillResignActive(_ scene: UIScene) { gpui_ios_will_resign_active(nil) }
    // A Face ID sheet only resigns active; entering the background ends the session it unlocked.
    func sceneDidEnterBackground(_ scene: UIScene) { sjel_ios_entered_background() }
}

/// `ios/src/lib.rs` asks for Face ID, or the passcode as its fallback, before a terminal on the
/// Mac opens or reattaches. The answer goes back on the main queue.
@_cdecl("sjel_ios_unlock")
public func sjelIOSUnlock() {
    LAContext().evaluatePolicy(.deviceOwnerAuthentication,
                               localizedReason: "Open a terminal on your Mac.") { ok, _ in
        DispatchQueue.main.async { sjel_ios_unlocked(ok) }
    }
}

final class SjelController: UIViewController {
    private var gpuiWindow: UnsafeMutableRawPointer!
    private var content: UIViewController!
    private var displayLink: CADisplayLink?
    private var discovery: Discovery?

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "Sjel"
        navigationController?.navigationBar.prefersLargeTitles = true
        view.backgroundColor = .systemBackground
        gpui_ios_set_embedded()
        gpui_ios_register_app()
        gpui_ios_run_demo()
        // The notification centre's delegate, so a reminder is shown while the app is open and a tap
        // on one reaches Rust (`Notifications.swift`). Local only: no push registration here.
        Reminders.shared.configure()
        // Browses for the Mac while the app is open; every answer goes to Rust (Discovery.swift).
        let discovery = Discovery()
        discovery.start()
        self.discovery = discovery
        guard let window = gpui_ios_get_window(), let controller = gpui_ios_view_controller(window) else {
            fatalError("GPUI did not create its view")
        }
        gpuiWindow = window
        content = Unmanaged<UIViewController>.fromOpaque(controller).takeUnretainedValue()
        addChild(content)
        view.addSubview(content.view)
        content.didMove(toParent: self)
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        let frame = view.safeAreaLayoutGuide.layoutFrame
        guard frame.width > 0, content.view.frame != frame else { return }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        content.view.frame = frame
        gpui_ios_layout_view(gpuiWindow)
        _ = gpui_ios_request_frame(gpuiWindow)
        CATransaction.commit()
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        displayLink = CADisplayLink(target: self, selector: #selector(frame))
        displayLink?.add(to: .main, forMode: .common)
    }

    @objc private func frame() { _ = gpui_ios_request_frame(gpuiWindow) }
}
