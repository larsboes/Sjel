import ServiceManagement
import SwiftUI
import SjelRelay

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

    private let bridge = NodeBridge()
    private let login = DashboardLogin()

    var statusImageName: String {
        nodeStatus.isReachable ? "circle.inset.filled" : "circle.dotted"
    }

    init() {
        Task {
            await refreshStatus()
        }
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
