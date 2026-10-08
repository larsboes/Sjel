import ServiceManagement
import SwiftUI
import SjelRelay
import UserNotifications

@main
struct SjelMacApp: App {
    @StateObject private var model = SjelMacViewModel()

    var body: some Scene {
        MenuBarExtra {
            SjelMenuBarView(model: model)
        } label: {
            Image(nsImage: MenuBarIcon.image(model.menuBarState))
        }
        .menuBarExtraStyle(.window)
    }
}

@MainActor
final class SjelMacViewModel: ObservableObject {
    @Published var status: NodeStatus = .offline
    /// When `status` was last read, so a panel that stopped updating says so.
    @Published var checkedAt: Date? = nil
    /// What the last "Open Dashboard" could not do, in words for the person who clicked it.
    @Published var openProblem: String? = nil
    @Published var launchesAtLogin: Bool = SMAppService.mainApp.status == .enabled

    /// Agent writes that wait for the owner in ask mode (ISA F10).
    @Published var pendingWrites: [AgentApproval] = []
    /// The agents' recent calls through the gate, newest first.
    @Published var agentCalls: [AgentCallRecord] = []
    /// Today's and tomorrow's calendar entries still ahead; nil when the calendar did not answer.
    @Published var today: [CalendarEntry]? = nil
    /// Capabilities with a start or stop in flight, so their buttons wait.
    @Published var busy: Set<String> = []

    /// One Keychain read shared by every poll (see `CachedToken`).
    private let token = CachedToken()
    private lazy var bridge = NodeBridge(token: token)
    private lazy var approvals = AgentApprovals(token: token)
    private let login = DashboardLogin()
    private let notifications = ApprovalNotifications()
    private var announced: Set<String> = []
    private var quickAsk: QuickAsk?

    /// The menu bar icon is the only part seen without a click, so it carries the state:
    /// something needs you (an agent waits, or a capability is down), Sjel is not running,
    /// or all is well. The panel says which.
    var menuBarState: MenuBarState {
        if !pendingWrites.isEmpty { return .attention }
        switch status {
        case .offline: return .offline
        case .known(let health) where !health.down.isEmpty: return .attention
        default: return .ok
        }
    }

    init() {
        quickAsk = QuickAsk { [weak self] question in self?.ask(question) }
        notifications.onDecision = { [weak self] id, allow in
            Task { @MainActor in await self?.decide(id: id, allow: allow) }
        }
        notifications.start()
        // Every five seconds: an agent waiting on Allow is waiting on this.
        Task { [weak self] in
            while !Task.isCancelled {
                await self?.refreshPending()
                try? await Task.sleep(for: .seconds(5))
            }
        }
        // The health asks every capability, so it polls slower. Opening the panel also
        // refreshes it, so what the panel shows is never older than one open.
        Task { [weak self] in
            while !Task.isCancelled {
                await self?.refreshStatus()
                try? await Task.sleep(for: .seconds(30))
            }
        }
    }

    func refreshPending() async {
        let view = await approvals.view()
        let pending = view.pending
        pendingWrites = pending
        agentCalls = view.calls
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
        status = await bridge.status()
        checkedAt = Date()
        today = await bridge.calendarToday().map { Today.ahead($0, now: Date()) }
    }

    /// Starts or stops capabilities one after the other, then reads the health again.
    func setRunning(_ names: [String], _ running: Bool) {
        let names = names.filter { !busy.contains($0) }
        busy.formUnion(names)
        Task {
            for name in names {
                if let problem = await bridge.setRunning(name, running) { openProblem = problem }
                busy.remove(name)
            }
            await refreshStatus()
        }
    }

    /// Opens the dashboard logged in: a single-use ticket from the shell, then the browser
    /// (ISA ISC-45). The shell's listener refuses a browser with no session.
    func showAsk() { quickAsk?.show() }

    /// The question goes to the dashboard's Ask drawer. Encoded here with the strict set, so
    /// an `&` or `#` in the question stays part of it.
    func ask(_ question: String) {
        var allowed = CharacterSet.alphanumerics
        allowed.insert(charactersIn: "-._~")
        let encoded = question.addingPercentEncoding(withAllowedCharacters: allowed) ?? ""
        openDashboard(next: "/?ask=\(encoded)")
    }

    func openDashboard(next: String? = nil) {
        Task {
            do throws(DashboardLogin.Failure) {
                let url = try await login.loginURL(next: next)
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

enum MenuBarState { case ok, attention, offline }

/// The hedgehog (`Resources/MenuBarIcon.pdf`, from `brand/sjel-hedgehog.svg`) as a template
/// image, so macOS tints it for a light or dark menu bar. One colour only, so the state is a
/// shape: a dot for attention, a faded mark for offline.
enum MenuBarIcon {
    /// Absent under `swift run`, which has no app bundle: an SF Symbol stands in.
    private static let mark = Bundle.main.image(forResource: "MenuBarIcon")
    private static let height: CGFloat = 16
    private static let badge: CGFloat = 6

    static func image(_ state: MenuBarState) -> NSImage {
        let label = switch state {
        case .ok: "Sjel"
        case .attention: "Sjel needs attention"
        case .offline: "Sjel is not running"
        }
        guard let mark else {
            let name = switch state {
            case .ok: "circle.inset.filled"
            case .attention: "exclamationmark.circle"
            case .offline: "circle.dotted"
            }
            return NSImage(systemSymbolName: name, accessibilityDescription: label) ?? NSImage()
        }
        let width = (mark.size.width / mark.size.height * height).rounded()
        let size = NSSize(width: width + (state == .attention ? 2 : 0), height: height)
        let image = NSImage(size: size, flipped: false) { _ in
            mark.draw(
                in: NSRect(x: 0, y: 0, width: width, height: height),
                from: .zero,
                operation: .sourceOver,
                fraction: state == .offline ? 0.35 : 1
            )
            if state == .attention {
                // Clear a ring first, so the dot reads apart from the spines under it.
                let dot = NSRect(x: size.width - badge, y: size.height - badge, width: badge, height: badge)
                NSGraphicsContext.current?.compositingOperation = .clear
                NSBezierPath(ovalIn: dot.insetBy(dx: -1.5, dy: -1.5)).fill()
                NSGraphicsContext.current?.compositingOperation = .sourceOver
                NSColor.black.setFill()
                NSBezierPath(ovalIn: dot).fill()
            }
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = label
        return image
    }
}

/// Answers one question: is Sjel all right, and does anything need me? What needs a decision
/// comes first, then what is broken, then the one action. Settings sit behind the ⋯ menu
/// (the explicitness ladder, Packs/design/skills/ui-craftsmanship/SKILL.md, rule 2).
struct SjelMenuBarView: View {
    @ObservedObject var model: SjelMacViewModel

    /// More down than this and the rest is a count, so the panel keeps its size.
    private let downShown = 5

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            header

            if !model.pendingWrites.isEmpty {
                Divider()
                approvals
            }

            if let today = model.today {
                Divider()
                todaySection(today)
            }

            Divider()
            statusSection

            if !model.agentCalls.isEmpty {
                Divider()
                agentActivity
            }

            if let problem = model.openProblem {
                Text(problem)
                    .font(.system(size: 11))
                    .foregroundStyle(.orange)
            }

            HStack {
                Button {
                    model.openDashboard()
                } label: {
                    Text("Open Dashboard").frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut("d", modifiers: .command)
                .help("Open the dashboard in the browser, logged in (⌘D)")

                Button("Ask") { model.showAsk() }
                    .help("Ask Sjel from any app with ⌃⌥Space")
            }
        }
        .padding(14)
        .frame(width: 280)
        .task { await model.refreshStatus() }
    }

    private var header: some View {
        HStack(alignment: .firstTextBaseline) {
            Text("Sjel")
                .font(.system(size: 13, weight: .semibold))
            if case .known(let health) = model.status {
                Text("\(health.version) · up \(uptime(health.uptimeSeconds))")
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Menu {
                Toggle(
                    "Open at Login",
                    isOn: Binding(
                        get: { model.launchesAtLogin },
                        set: { model.setLaunchesAtLogin($0) }
                    )
                )
                Divider()
                Button("Quit Sjel") { NSApplication.shared.terminate(nil) }
                    .keyboardShortcut("q", modifiers: .command)
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .help("Settings and Quit")
        }
    }

    private var approvals: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Waiting for you")
                .font(.system(size: 11, weight: .semibold))
            ForEach(model.pendingWrites) { approval in
                VStack(alignment: .leading, spacing: 4) {
                    Text(approval.summary)
                        .font(.system(size: 11, design: .monospaced))
                        .lineLimit(2)
                        .help(approval.preview)
                    HStack {
                        Button("Allow") { Task { await model.decide(id: approval.id, allow: true) } }
                        Button("Deny") { Task { await model.decide(id: approval.id, allow: false) } }
                    }
                    .controlSize(.small)
                }
            }
        }
    }

    @ViewBuilder
    private var statusSection: some View {
        switch model.status {
        case .offline:
            line(color: .secondary, "Sjel is not running on this Mac.")
        case .alive:
            line(color: .green, "Running")
            Text("Capability health needs the deployment token in the Keychain.")
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
        case .known(let health):
            let down = health.down
            let total = health.capabilities.count
            if down.isEmpty {
                line(color: .green, "All \(total) capabilities up")
            } else {
                HStack {
                    line(color: .orange, "\(down.count) of \(total) capabilities down")
                    Spacer()
                    if down.count > 1 {
                        Button("Start all") { model.setRunning(down, true) }
                            .controlSize(.small)
                            .help("Start every capability that is down")
                    }
                }
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(down.prefix(downShown), id: \.self) { name in
                        CapabilityRow(name: name, up: false, busy: model.busy.contains(name)) {
                            model.setRunning([name], true)
                        }
                    }
                    if down.count > downShown {
                        Text("and \(down.count - downShown) more")
                            .font(.system(size: 11, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .help(down.dropFirst(downShown).joined(separator: ", "))
                    }
                }
                .padding(.leading, 14)
            }
            allCapabilities(health)
        }
        // Stale: the poll runs every 30 s, so anything older means it stopped.
        if let checked = model.checkedAt, Date().timeIntervalSince(checked) > 90 {
            Text("Checked \(checked, style: .relative) ago")
                .font(.system(size: 10))
                .foregroundStyle(.secondary)
        }
    }

    /// The calendar for today and tomorrow. An entry still marked possible carries a dot:
    /// Home ranks it as a decision. A click opens the calendar.
    private func todaySection(_ entries: [CalendarEntry]) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Today").font(.system(size: 11, weight: .semibold))
            if entries.isEmpty {
                Text("Nothing on the calendar until tomorrow night.")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
            }
            ForEach(entries.prefix(4)) { entry in
                Button {
                    model.openDashboard(next: "/calendar")
                } label: {
                    HStack(spacing: 6) {
                        Circle()
                            .fill(entry.isPossible ? Color.orange : Color.clear)
                            .frame(width: 6, height: 6)
                        Text(entry.title).lineLimit(1).truncationMode(.tail)
                        Spacer()
                        Text(Today.when(entry, now: Date()))
                            .font(.system(size: 10, design: .monospaced))
                            .foregroundStyle(.secondary)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .font(.system(size: 11))
                .help(entry.isPossible ? "\(entry.title). Still possible: decide in the calendar." : entry.title)
            }
            if entries.count > 4 {
                Text("and \(entries.count - 4) more")
                    .font(.system(size: 10))
                    .foregroundStyle(.secondary)
            }
        }
    }

    /// Every capability, folded away: stopping one is rare, so it sits a click down.
    private func allCapabilities(_ health: NodeHealth) -> some View {
        DisclosureGroup("All capabilities") {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(health.capabilities.keys.sorted(), id: \.self) { name in
                    let up = health.capabilities[name]?.up ?? false
                    // Stopping the shell would take this panel's own source with it.
                    CapabilityRow(name: name, up: up, busy: model.busy.contains(name), canStop: name != "sjel-status") {
                        model.setRunning([name], !up)
                    }
                }
            }
            .padding(.top, 4)
        }
        .font(.system(size: 11))
        .foregroundStyle(.secondary)
    }

    /// Reads are most of the log and say little, so they are a count. Anything else (a
    /// write, a refusal) is listed.
    private var agentActivity: some View {
        let hourAgo = Date().addingTimeInterval(-3600)
        let lastHour = model.agentCalls.filter { $0.date > hourAgo }
        let reads = lastHour.filter(\.isRead).count
        let notable = model.agentCalls.filter { !$0.isRead }.prefix(3)
        return VStack(alignment: .leading, spacing: 4) {
            HStack(alignment: .firstTextBaseline) {
                Text("Agents").font(.system(size: 11, weight: .semibold))
                Spacer()
                if let last = model.agentCalls.first {
                    Text("last call \(last.date, style: .relative) ago")
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                }
            }
            Text("This hour: \(reads) reads, \(lastHour.count - reads) other")
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
            ForEach(Array(notable)) { call in
                HStack {
                    Text("\(call.capability) \(call.method) \(call.path)")
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Spacer()
                    Text(call.decision).foregroundStyle(call.status >= 400 ? .orange : .secondary)
                }
                .font(.system(size: 10, design: .monospaced))
                .help("\(call.capability) \(call.method) \(call.path) → \(call.status), \(call.decision), \(call.date.formatted(date: .omitted, time: .shortened))")
            }
        }
    }

    private func line(color: Color, _ text: String) -> some View {
        HStack(spacing: 6) {
            Circle().fill(color).frame(width: 8, height: 8)
            Text(text).font(.system(size: 12, weight: .medium))
        }
    }

    private func uptime(_ seconds: UInt64) -> String {
        let minutes = seconds / 60
        if minutes < 60 { return "\(minutes)m" }
        let hours = minutes / 60
        if hours < 48 { return "\(hours)h" }
        return "\(hours / 24)d"
    }
}

/// One capability: its name, and the one action that changes its state. The list that holds
/// up capabilities is folded away already, so Stop needs no second hiding.
struct CapabilityRow: View {
    let name: String
    let up: Bool
    let busy: Bool
    var canStop = true
    let action: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            Circle().fill(up ? Color.green : Color.orange).frame(width: 6, height: 6)
            Text(name).font(.system(size: 11, design: .monospaced))
            Spacer()
            if busy {
                ProgressView().controlSize(.mini)
            } else if !up || canStop {
                Button(up ? "Stop" : "Start", action: action)
                    .buttonStyle(.borderless)
                    .font(.system(size: 11))
                    .help(up ? "Stop \(name). It stays stopped until you start it." : "Start \(name)")
            }
        }
        .frame(height: 18)
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
