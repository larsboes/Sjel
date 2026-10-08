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
        openProblem = nil
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

/// Answers one question: is Sjel all right, and does anything need me?
///
/// Layout rule: a click or a poll must not move what the reader is looking at. The fixed parts
/// come first and keep their place in every state: the actions, the message line, the summary,
/// and every capability in a list of fixed height. The parts that come and go (waiting writes,
/// the calendar, agent activity) sit below them, so their arrival or departure cannot push a
/// button or a capability. Settings sit behind the ⋯ menu (the explicitness ladder,
/// ui-craftsmanship rule 2).
struct SjelMenuBarView: View {
    @ObservedObject var model: SjelMacViewModel

    /// Capability rows in view at once. The list keeps this height in every state, so a status
    /// change never moves what sits below it. More than this scrolls.
    private let rowsVisible = 6
    private let rowHeight: CGFloat = 20
    /// Older than this and the header says how old (the poll runs every 30 s).
    private let staleAfter: TimeInterval = 90

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            header
            actions
            message
            Divider()
            statusSummary
            capabilityList

            if !model.pendingWrites.isEmpty {
                Divider()
                approvals
            }
            if let today = model.today {
                Divider()
                todaySection(today)
            }
            if !model.agentCalls.isEmpty {
                Divider()
                agentActivity
            }
        }
        .padding(14)
        .frame(width: 280)
        .task { await model.refreshStatus() }
    }

    /// The health the list shows, or nil when the shell did not give one.
    private var health: NodeHealth? {
        if case .known(let health) = model.status { return health }
        return nil
    }

    private var downNames: [String] { health?.down ?? [] }

    private var header: some View {
        HStack(alignment: .firstTextBaseline) {
            Text("Sjel")
                .font(.system(size: 13, weight: .semibold))
            if let health {
                Text("\(health.version) · up \(uptime(health.uptimeSeconds))")
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                if let checked = model.checkedAt, Date().timeIntervalSince(checked) > staleAfter {
                    Text("· checked \(checked, style: .relative) ago")
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
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

    /// Always both buttons, always here: nothing above them comes and goes.
    private var actions: some View {
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

    /// What the last action could not do. Its line is always reserved, so an error appearing
    /// or clearing moves nothing.
    private var message: some View {
        Text(model.openProblem ?? "")
            .font(.system(size: 11))
            .foregroundStyle(.orange)
            .lineLimit(1)
            .truncationMode(.tail)
            .frame(maxWidth: .infinity, minHeight: 14, maxHeight: 14, alignment: .leading)
            .help(model.openProblem ?? "")
    }

    private var summaryLine: (tone: Color, text: String) {
        switch model.status {
        case .offline:
            return (.secondary, "Sjel is not running on this Mac.")
        case .alive:
            return (.green, "Running")
        case .known(let health):
            let down = health.down
            let total = health.capabilities.count
            if down.isEmpty { return (.green, "All \(total) capabilities up") }
            return (.orange, "\(down.count) of \(total) down: \(down.joined(separator: ", "))")
        }
    }

    /// One line, always the same height. The count and the names change; the place does not.
    private var statusSummary: some View {
        let line = summaryLine
        return HStack(spacing: 6) {
            Circle().fill(line.tone).frame(width: 8, height: 8)
            Text(line.text)
                .font(.system(size: 12, weight: .medium))
                .lineLimit(1)
                .truncationMode(.tail)
                .help(line.text)
            Spacer(minLength: 6)
            startAll
        }
        .frame(height: 20)
    }

    /// Always in place. Disabled when nothing is down, and the tip says why.
    private var startAll: some View {
        Button("Start all") { model.setRunning(downNames, true) }
            .controlSize(.small)
            .disabled(downNames.isEmpty)
            .help(
                downNames.isEmpty
                    ? (health == nil ? "Sjel's health is not known yet" : "Every capability is up")
                    : "Start every capability that is down"
            )
            .frame(width: 66, alignment: .trailing)
    }

    /// Every capability, in name order. A start or a stop changes one row's dot and button, and
    /// no row leaves the list. Fixed height, so the panel is the same size in every state.
    private var capabilityList: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                if let health {
                    ForEach(health.rowOrder, id: \.self) { name in
                        let up = health.capabilities[name]?.up ?? false
                        // Stopping the shell would take this panel's own source with it.
                        CapabilityRow(name: name, up: up, busy: model.busy.contains(name), canStop: name != "sjel-status") {
                            model.setRunning([name], !up)
                        }
                    }
                } else if case .alive = model.status {
                    Text("Capability health needs the deployment token in the Keychain.")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .frame(height: CGFloat(rowsVisible) * rowHeight)
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

    private func uptime(_ seconds: UInt64) -> String {
        let minutes = seconds / 60
        if minutes < 60 { return "\(minutes)m" }
        let hours = minutes / 60
        if hours < 48 { return "\(hours)h" }
        return "\(hours / 24)d"
    }
}

/// One capability: its dot, its name, and the one action that changes its state. The action
/// keeps the same width whatever it holds, so a spinner or a label change moves nothing.
struct CapabilityRow: View {
    let name: String
    let up: Bool
    let busy: Bool
    var canStop = true
    let action: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            Circle().fill(up ? Color.green : Color.orange).frame(width: 6, height: 6)
            Text(name)
                .font(.system(size: 11, design: .monospaced))
                .lineLimit(1)
            Spacer(minLength: 4)
            Group {
                if busy {
                    ProgressView().controlSize(.mini)
                } else if up && !canStop {
                    Button("Stop") {}
                        .disabled(true)
                        .help("The shell serves this panel. Stop it from a terminal.")
                } else {
                    Button(up ? "Stop" : "Start", action: action)
                        .help(up ? "Stop \(name). It stays stopped until you start it." : "Start \(name)")
                }
            }
            .buttonStyle(.borderless)
            .font(.system(size: 11))
            .frame(width: 44, alignment: .trailing)
        }
        .frame(height: 20)
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
