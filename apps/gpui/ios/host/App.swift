import LocalAuthentication
import UIKit

// UIKit owns the window and safe areas; NSH renders the phone screens.
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
        // The phone was in a pocket and the day moved on: the reminders it holds are the ones it
        // scheduled last time it was awake (`ios/src/notify.rs` rebuilds them).
        sjel_ios_became_active()
    }
    // A Face ID sheet only resigns active; entering the background ends the session it unlocked.
    func sceneDidEnterBackground(_ scene: UIScene) {
        terminalUnlockEpoch &+= 1
        terminalUnlockContext?.invalidate()
        terminalUnlockContext = nil
        sjel_ios_entered_background()
    }
}

/// `ios/src/events.rs` asks for Face ID, or the passcode as its fallback, before a terminal on the
/// Mac opens or reattaches. The answer goes back on the main queue.
private var terminalUnlockEpoch: UInt64 = 0
private var terminalUnlockContext: LAContext?

@_cdecl("sjel_ios_unlock")
public func sjelIOSUnlock() {
    DispatchQueue.main.async {
        guard UIApplication.shared.applicationState != .background else {
            sjel_ios_unlocked(false)
            return
        }
        terminalUnlockEpoch &+= 1
        let epoch = terminalUnlockEpoch
        terminalUnlockContext?.invalidate()
        let context = LAContext()
        terminalUnlockContext = context
        context.evaluatePolicy(.deviceOwnerAuthentication,
                               localizedReason: "Open a terminal on your Mac.") { ok, _ in
            DispatchQueue.main.async {
                guard epoch == terminalUnlockEpoch else { return }
                terminalUnlockContext = nil
                sjel_ios_unlocked(ok)
            }
        }
    }
}

/// The full-screen NSH background, including the bands outside the safe areas.
private weak var nshScreens: UIView?

/// Paints the cover with the palette's page colours and forces a scheme: `style` 0 follows the
/// system, 1 is light, 2 is dark. The colours are `0xRRGGBB`.
@_cdecl("sjel_ios_nsh_cover")
public func sjelIOSNshCover(_ style: Int32, _ light: UInt32, _ dark: UInt32) {
    func color(_ rgb: UInt32) -> UIColor {
        UIColor(
            red: CGFloat((rgb >> 16) & 0xff) / 255,
            green: CGFloat((rgb >> 8) & 0xff) / 255,
            blue: CGFloat(rgb & 0xff) / 255,
            alpha: 1)
    }
    DispatchQueue.main.async {
        guard let cover = nshScreens else { return }
        cover.overrideUserInterfaceStyle =
            style == 2 ? .dark : style == 1 ? .light : .unspecified
        sjel_ios_nsh_apply_theme()
        cover.backgroundColor = UIColor { traits in
            traits.userInterfaceStyle == .dark ? color(dark) : color(light)
        }
    }
}

final class SjelController: UIViewController {
    private var discovery: Discovery?

    override func viewDidLoad() {
        super.viewDidLoad()
        // Each NSH page carries its own heading.
        navigationController?.setNavigationBarHidden(true, animated: false)
        view.backgroundColor = .systemBackground
        view.layoutIfNeeded()
        let cover = UIView(frame: view.bounds)
        cover.backgroundColor = .systemBackground
        view.addSubview(cover)
        cover.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            cover.topAnchor.constraint(equalTo: view.topAnchor),
            cover.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            cover.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            cover.trailingAnchor.constraint(equalTo: view.trailingAnchor),
        ])
        let screens = UIView(frame: view.safeAreaLayoutGuide.layoutFrame)
        screens.backgroundColor = .clear
        cover.addSubview(screens)
        screens.translatesAutoresizingMaskIntoConstraints = false
        let guide = cover.safeAreaLayoutGuide
        NSLayoutConstraint.activate([
            screens.topAnchor.constraint(equalTo: guide.topAnchor),
            screens.bottomAnchor.constraint(equalTo: guide.bottomAnchor),
            screens.leadingAnchor.constraint(equalTo: guide.leadingAnchor),
            screens.trailingAnchor.constraint(equalTo: guide.trailingAnchor),
        ])
        view.layoutIfNeeded()
        nshScreens = cover
        if !sjel_ios_nsh_mount(Unmanaged.passUnretained(screens).toOpaque()) {
            fatalError("NSH could not mount")
        }
        let pinch = UIPinchGestureRecognizer(target: self, action: #selector(pinched(_:)))
        pinch.cancelsTouchesInView = false
        screens.addGestureRecognizer(pinch)
        Reminders.shared.configure()
        let discovery = Discovery()
        discovery.start()
        self.discovery = discovery
    }

    @objc private func pinched(_ gesture: UIPinchGestureRecognizer) {
        sjel_ios_pinch(Double(gesture.scale), gesture.state == .ended || gesture.state == .cancelled || gesture.state == .failed)
    }
}
