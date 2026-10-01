import ServiceManagement
import SwiftUI
import SjelRelay
import UserNotifications

@main
struct SjelMacApp: App {
    @StateObject private var model = SjelMacViewModel()

    var body: some Scene {
        MenuBarExtra("Sjel", systemImage: model.statusImageName) {
            SjelMenuBarView(model: model)
        }
        .menuBarExtraStyle(.window)
    }
}

@MainActor
final class SjelMacViewModel: ObservableObject {
    @Published var nodeStatus: SjelNodeStatus = SjelNodeStatus(isReachable: false)
    /// What the last "Open Dashboard" could not do, in words for the person who clicked it.
    @Published var openProblem: String? = nil
    @Published var launchesAtLogin: Bool = SMAppService.mainApp.status == .enabled

    /// Agent writes that wait for the owner in ask mode (ISA F10).
    @Published var pendingWrites: [AgentApproval] = []

    private let bridge = NodeBridge()
    private let login = DashboardLogin()
    private let approvals = AgentApprovals()
    private let notifications = ApprovalNotifications()
    private var announced: Set<String> = []

    var statusImageName: String {
        nodeStatus.isReachable ? "circle.inset.filled" : "circle.dotted"
    }

    init() {
        notifications.onDecision = { [weak self] id, allow in
            Task { @MainActor in await self?.decide(id: id, allow: allow) }
        }
        notifications.start()
        Task {
            await refreshStatus()
        }
        // Every five seconds: an agent waiting on Allow is waiting on this.
        Task { [weak self] in
            while !Task.isCancelled {
                await self?.refreshPending()
                try? await Task.sleep(for: .seconds(5))
            }
        }
    }

    func refreshPending() async {
        let pending = await approvals.pending()
        pendingWrites = pending
        for approval in pending where !announced.contains(approval.id) {
            announced.insert(approval.id)
            notifications.announce(approval)
        }
    }

    func decide(id: String, allow: Bool) async {
        _ = await approvals.decide(id: id, allow: allow)
        notifications.withdraw(id: id)
        await refreshPending()
    }

    func refreshStatus() async {
        let status = await bridge.checkHealth()
        self.nodeStatus = status
    }

    /// Opens the dashboard logged in: a single-use ticket from the shell, then the browser
    /// (ISA ISC-45). The shell's listener refuses a browser with no session.
    func openDashboard() {
        Task {
            do throws(DashboardLogin.Failure) {
                let url = try await login.loginURL()
                openProblem = nil
                NSWorkspace.shared.open(url)
            } catch {
                openProblem = switch error {
                case .notSetUp: "Sjel is not set up on this Mac yet."
                case .unreachable: "Sjel is not running on this Mac."
                case .refused(let status): "Sjel refused the login (\(status))."
                }
            }
        }
    }

    /// Start this app when the user logs in. `SMAppService` works only from an app bundle,
    /// which is what `apps/mac/install` builds.
    func setLaunchesAtLogin(_ enabled: Bool) {
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
        } catch {
            openProblem = "Could not change the login item: \(error.localizedDescription)"
        }
        launchesAtLogin = SMAppService.mainApp.status == .enabled
    }
}

struct SjelMenuBarView: View {
    @ObservedObject var model: SjelMacViewModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            // Header
            HStack {
                Text("SJEL")
                    .font(.system(size: 13, weight: .bold, design: .monospaced))
                    .tracking(1.5)
                Spacer()
                Text(model.nodeStatus.version)
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundColor(.secondary)
            }
            .padding(.bottom, 2)

            Divider()

            // Node Status
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    Circle()
                        .fill(model.nodeStatus.isReachable ? Color.green : Color.orange)
                        .frame(width: 8, height: 8)
                    Text("Local Node")
                        .font(.system(size: 12, weight: .medium))
                    Spacer()
                    Text(model.nodeStatus.isReachable ? "\(Int(model.nodeStatus.latencyMs))ms" : "offline")
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundColor(.secondary)
                }
            }

            if !model.pendingWrites.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Agent asks to")
                        .font(.system(size: 11, weight: .semibold))
                    ForEach(model.pendingWrites) { approval in
                        VStack(alignment: .leading, spacing: 3) {
                            Text(approval.summary)
                                .font(.system(size: 11, design: .monospaced))
                                .lineLimit(2)
                            HStack {
                                Button("Allow") { Task { await model.decide(id: approval.id, allow: true) } }
                                Button("Deny") { Task { await model.decide(id: approval.id, allow: false) } }
                            }
                            .controlSize(.small)
                        }
                    }
                }
                Divider()
            }

            if let problem = model.openProblem {
                Text(problem)
                    .font(.system(size: 11))
                    .foregroundColor(.orange)
            }

            Toggle(
                "Open at login",
                isOn: Binding(
                    get: { model.launchesAtLogin },
                    set: { model.setLaunchesAtLogin($0) }
                )
            )
            .font(.system(size: 12))
            .toggleStyle(.switch)
            .controlSize(.small)

            Divider()

            // Actions
            HStack {
                Button("Open Dashboard") {
                    model.openDashboard()
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.small)

                Spacer()

                Button("Refresh") {
                    Task {
                        await model.refreshStatus()
                    }
                }
                .controlSize(.small)

                Button("Quit") {
                    NSApplication.shared.terminate(nil)
                }
                .controlSize(.small)
            }
        }
        .padding(14)
        .frame(width: 280)
    }
}

/// Posts one notification per waiting agent write, with Allow and Deny on it, and reports the
/// owner's choice back. Needs an app bundle, which `apps/mac/install` builds.
final class ApprovalNotifications: NSObject, UNUserNotificationCenterDelegate, @unchecked Sendable {
    private static let category = "SJEL_AGENT_WRITE"
    var onDecision: (@Sendable (String, Bool) -> Void)?

    func start() {
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        let allow = UNNotificationAction(identifier: "ALLOW", title: "Allow")
        let deny = UNNotificationAction(identifier: "DENY", title: "Deny", options: [.destructive])
        center.setNotificationCategories([
            UNNotificationCategory(identifier: Self.category, actions: [allow, deny], intentIdentifiers: []),
        ])
        center.requestAuthorization(options: [.alert, .sound]) { _, _ in }
    }

    func announce(_ approval: AgentApproval) {
        let content = UNMutableNotificationContent()
        content.title = "An agent asks to change something"
        content.body = approval.summary
        content.categoryIdentifier = Self.category
        content.sound = .default
        let request = UNNotificationRequest(identifier: approval.id, content: content, trigger: nil)
        UNUserNotificationCenter.current().add(request)
    }

    func withdraw(id: String) {
        UNUserNotificationCenter.current().removeDeliveredNotifications(withIdentifiers: [id])
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        let id = response.notification.request.identifier
        switch response.actionIdentifier {
        case "ALLOW": onDecision?(id, true)
        case "DENY": onDecision?(id, false)
        default: break
        }
        completionHandler()
    }

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        completionHandler([.banner, .sound])
    }
}
