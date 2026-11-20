import UIKit
import os

enum PhoneGpuiRoute: Int32 {
    case documents = 0
    case today = 1
    case capture = 2
    case projects = 3
    case people = 4
    case inventory = 5
    case finance = 6
    case calendar = 7
    case settings = 8
    case scratchpad = 9
    case activity = 10
    case soundscape = 11
    case mail = 12
    case agent = 13
    case travel = 14
    case map = 15
    case interior = 16
    case terminal = 17

    var identifier: String {
        switch self {
        case .documents: "documents-gpui"
        case .today: "today-gpui"
        case .capture: "capture-gpui"
        case .projects: "projects-gpui"
        case .people: "people-gpui"
        case .inventory: "inventory-gpui"
        case .finance: "finance-gpui"
        case .calendar: "calendar-gpui"
        case .settings: "settings-gpui"
        case .scratchpad: "scratchpad-gpui"
        case .activity: "activity-gpui"
        case .soundscape: "soundscape-gpui"
        case .mail: "mail-gpui"
        case .agent: "agent-gpui"
        case .travel: "travel-gpui"
        case .map: "map-gpui"
        case .interior: "interior-gpui"
        case .terminal: "terminal-gpui"
        }
    }

    var label: String {
        switch self {
        case .documents: "Documents"
        case .today: "TODAY"
        case .capture: "Capture"
        case .projects: "Projects"
        case .people: "People & Places"
        case .inventory: "Inventory & Wishlist"
        case .finance: "Finance"
        case .calendar: "Calendar"
        case .settings: "Settings"
        case .scratchpad: "Daily Scratchpad"
        case .activity: "Activity"
        case .soundscape: "Soundscape"
        case .mail: "Mail"
        case .agent: "Agent"
        case .travel: "Travel"
        case .map: "Map & Places"
        case .interior: "Interior"
        case .terminal: "Terminal"
        }
    }
}

private weak var activeSurface: GpuiSurface?
private let gpuiLog = Logger(subsystem: "dev.sjel.ios", category: "GPUI")

func activeGpuiController(for route: PhoneGpuiRoute) -> UIViewController? {
    guard activeSurface?.accessibilityIdentifier == route.identifier else { return nil }
    return activeSurface?.owningController()
}

final class GpuiSurface: UIView {
    private var gpuiWindow: UnsafeMutableRawPointer?
    private weak var content: UIViewController?
    private var displayLink: CADisplayLink?

    init() {
        super.init(frame: .zero)
        registerForTraitChanges([UITraitUserInterfaceStyle.self]) { (surface: GpuiSurface, _) in
            if activeSurface === surface {
                sjel_ios_gpui_apply_theme(surface.traitCollection.userInterfaceStyle == .dark)
            }
        }
    }

    required init?(coder: NSCoder) { nil }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window == nil {
            isAccessibilityElement = false
            NotificationCenter.default.removeObserver(self)
            displayLink?.invalidate()
            displayLink = nil
            if activeSurface === self { activeSurface = nil }
            if let content, content.view.superview === self {
                gpui_ios_will_resign_active(nil)
                content.view.endEditing(true)
                content.willMove(toParent: nil)
                content.view.removeFromSuperview()
                content.removeFromParent()
            }
            content = nil
        } else {
            setNeedsLayout()
        }
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard window != nil, bounds.width > 0, bounds.height > 0 else { return }
        if content == nil {
            guard let host = owningController(),
                  let gpuiWindow = sjel_ios_gpui_start(),
                  let controller = gpui_ios_view_controller(gpuiWindow) else {
                showUnavailable()
                return
            }
            subviews.forEach { $0.removeFromSuperview() }
            let child = Unmanaged<UIViewController>.fromOpaque(controller).takeUnretainedValue()
            child.willMove(toParent: nil)
            child.view.removeFromSuperview()
            child.removeFromParent()
            host.addChild(child)
            addSubview(child.view)
            child.didMove(toParent: host)
            content = child
            self.gpuiWindow = gpuiWindow
            activeSurface = self
            sjel_ios_gpui_apply_theme(traitCollection.userInterfaceStyle == .dark)
            let link = CADisplayLink(target: self, selector: #selector(drawFrame))
            link.add(to: .main, forMode: .common)
            displayLink = link
            let notifications = NotificationCenter.default
            notifications.addObserver(self, selector: #selector(becameActive(_:)),
                                      name: UIApplication.didBecomeActiveNotification, object: nil)
            notifications.addObserver(self, selector: #selector(resignedActive(_:)),
                                      name: UIApplication.willResignActiveNotification, object: nil)
            if UIApplication.shared.applicationState == .active {
                gpui_ios_did_become_active(nil)
            } else {
                link.isPaused = true
            }
        }
        guard let gpuiWindow, let content else { return }
        if content.view.frame != bounds {
            content.view.frame = bounds
            gpui_ios_layout_view(gpuiWindow)
        }
        _ = gpui_ios_request_frame(gpuiWindow)
        updateRoute()
    }

    @objc private func becameActive(_ notification: Notification) {
        gpui_ios_did_become_active(nil)
        displayLink?.isPaused = false
    }

    @objc private func resignedActive(_ notification: Notification) {
        displayLink?.isPaused = true
        gpui_ios_will_resign_active(nil)
    }

    @objc private func drawFrame() {
        guard UIApplication.shared.applicationState == .active,
              let gpuiWindow, content?.view.superview === self else { return }
        _ = gpui_ios_request_frame(gpuiWindow)
        updateRoute()
    }

    func owningController() -> UIViewController? {
        var responder: UIResponder? = next
        while let current = responder {
            if let controller = current as? UIViewController { return controller }
            responder = current.next
        }
        return nil
    }

    private func updateRoute() {
        guard let route = PhoneGpuiRoute(rawValue: sjel_ios_gpui_current_route()),
              accessibilityIdentifier != route.identifier else { return }
        accessibilityIdentifier = route.identifier
        accessibilityLabel = route.label
        isAccessibilityElement = true
        gpuiLog.notice("Mounted \(route.identifier, privacy: .public)")
    }

    private func showUnavailable() {
        guard subviews.isEmpty else { return }
        gpuiLog.error("Could not mount the phone GPUI window")
        let message = UILabel(frame: bounds)
        message.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        message.textAlignment = .center
        message.text = "Sjel could not start."
        addSubview(message)
    }
}
