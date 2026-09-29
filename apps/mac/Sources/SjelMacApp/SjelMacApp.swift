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
    @Published var isRelayActive: Bool = true
    @Published var lastRelaySync: Date? = nil

    private let bridge = NodeBridge()
    private let relay = CloudKitRelay()

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

    func openDashboard() {
        if let url = URL(string: "http://127.0.0.1:8082") {
            NSWorkspace.shared.open(url)
        }
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

            // CloudKit Relay Status
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    Image(systemName: "lock.shield.fill")
                        .font(.system(size: 10))
                        .foregroundColor(.blue)
                    Text("CloudKit E2EE Relay")
                        .font(.system(size: 12, weight: .medium))
                    Spacer()
                    Text("ACTIVE")
                        .font(.system(size: 9, weight: .bold, design: .monospaced))
                        .padding(.horizontal, 4)
                        .padding(.vertical, 2)
                        .background(Color.blue.opacity(0.15))
                        .cornerRadius(3)
                        .foregroundColor(.blue)
                }

                Text("Product Rule 4: zero plaintext C2 leaves host")
                    .font(.system(size: 10))
                    .foregroundColor(.secondary)
            }

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
