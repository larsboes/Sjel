import AppKit
import Carbon.HIToolbox
import SjelRelay
import SwiftUI

/// One turn displayed in the floating QuickAsk session.
@MainActor
final class QuickAskSession: ObservableObject {
    struct Entry: Identifiable, Equatable {
        let id = UUID()
        let role: String
        let content: String
        let meta: String?
    }

    @Published var entries: [Entry] = []
    @Published var busy: Bool = false
    var lastQuestion: String? {
        entries.last(where: { $0.role == "user" })?.content
    }

    func clear() {
        entries.removeAll()
        busy = false
    }
}

/// ⌥Space (or ⌃⌥Space) from any app opens a floating multi-turn session. Enter answers right
/// inside the floating panel from `POST /assistant/api/ask` (and live capabilities), so the
/// conversation stays in-session. "Continue in Sjel Desktop" (`⌘O`) hands the session to the
/// native GPUI app via `sjel://ask?q=…`.
@MainActor
final class QuickAsk {
    private var optionSpaceHotKey: EventHotKeyRef?
    private var controlOptionSpaceHotKey: EventHotKeyRef?
    private var handler: EventHandlerRef?
    private var panel: NSPanel?
    private var resignObserver: NSObjectProtocol?
    let session = QuickAskSession()
    private let askInSession: (String, [AskTurn]) async -> AskOutcome
    private let openDesktop: (String?) -> Void
    private let openDashboard: (String?) -> Void

    init(
        askInSession: @escaping (String, [AskTurn]) async -> AskOutcome,
        openDesktop: @escaping (String?) -> Void,
        openDashboard: @escaping (String?) -> Void
    ) {
        self.askInSession = askInSession
        self.openDesktop = openDesktop
        self.openDashboard = openDashboard
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        let me = Unmanaged.passUnretained(self).toOpaque()
        InstallEventHandler(GetApplicationEventTarget(), { _, _, context in
            guard let context else { return noErr }
            let quickAsk = Unmanaged<QuickAsk>.fromOpaque(context).takeUnretainedValue()
            // Carbon delivers hot keys on the main run loop.
            MainActor.assumeIsolated { quickAsk.toggle() }
            return noErr
        }, 1, &spec, me, &handler)
        let optId = EventHotKeyID(signature: OSType(0x534A_454C), id: 1) // "SJEL" ⌥Space
        RegisterEventHotKey(UInt32(kVK_Space), UInt32(optionKey), optId, GetApplicationEventTarget(), 0, &optionSpaceHotKey)
        let ctrlOptId = EventHotKeyID(signature: OSType(0x534A_454C), id: 2) // "SJEL" ⌃⌥Space
        RegisterEventHotKey(UInt32(kVK_Space), UInt32(controlKey | optionKey), ctrlOptId, GetApplicationEventTarget(), 0, &controlOptionSpaceHotKey)
    }

    func toggle() {
        if let panel, panel.isVisible {
            close()
        } else {
            show()
        }
    }

    func show(initialQuestion: String? = nil) {
        let panel = panel ?? makePanel()
        self.panel = panel
        resizePanel(expanded: !session.entries.isEmpty || initialQuestion != nil)
        if !panel.isVisible {
            panel.center()
        }
        NSApp.activate(ignoringOtherApps: true)
        panel.makeKeyAndOrderFront(nil)
        panel.orderFrontRegardless()
        if let q = initialQuestion?.trimmingCharacters(in: .whitespacesAndNewlines), !q.isEmpty {
            submit(q)
        }
    }

    func submit(_ question: String) {
        let trimmed = question.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !session.busy else { return }
        let history: [AskTurn] = session.entries.compactMap { entry in
            guard entry.role == "user" || entry.role == "assistant" else { return nil }
            return AskTurn(role: entry.role, content: entry.content)
        }
        session.entries.append(.init(role: "user", content: trimmed, meta: nil))
        session.busy = true
        resizePanel(expanded: true)
        Task { @MainActor [weak self] in
            guard let self else { return }
            let result = await self.askInSession(trimmed, history)
            self.session.busy = false
            switch result {
            case .reply(let reply):
                let meta = reply.tools.isEmpty
                    ? "\(reply.model) · \(reply.source)"
                    : "\(reply.model) · tools: \(reply.tools.joined(separator: ", "))"
                self.session.entries.append(.init(role: "assistant", content: reply.text, meta: meta))
            case .failure(let error):
                self.session.entries.append(.init(role: "error", content: error, meta: nil))
            }
        }
    }

    private func resizePanel(expanded: Bool) {
        guard let panel else { return }
        let targetHeight: CGFloat = expanded ? 340 : 56
        var frame = panel.frame
        let delta = targetHeight - frame.height
        guard abs(delta) > 1 else { return }
        frame.origin.y -= delta
        frame.size.height = targetHeight
        panel.setFrame(frame, display: true, animate: panel.isVisible)
    }

    private func close() {
        panel?.orderOut(nil)
    }

    private func makePanel() -> NSPanel {
        let panel = KeyPanel(
            contentRect: NSRect(x: 0, y: 0, width: 560, height: 56),
            styleMask: [.titled, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        panel.titleVisibility = .hidden
        panel.titlebarAppearsTransparent = true
        panel.isMovableByWindowBackground = true
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]
        panel.hidesOnDeactivate = false
        panel.standardWindowButton(.closeButton)?.isHidden = true
        panel.standardWindowButton(.miniaturizeButton)?.isHidden = true
        panel.standardWindowButton(.zoomButton)?.isHidden = true
        panel.contentView = NSHostingView(rootView: QuickAskField(
            session: session,
            onSubmit: { [weak self] text in
                self?.submit(text)
            },
            onContinueDesktop: { [weak self] draft in
                guard let self else { return }
                let q = draft.isEmpty ? self.session.lastQuestion : draft
                self.close()
                self.openDesktop(q)
            },
            onOpenDashboard: { [weak self] draft in
                guard let self else { return }
                let q = draft.isEmpty ? self.session.lastQuestion : draft
                self.close()
                self.openDashboard(q)
            },
            onClear: { [weak self] in
                self?.session.clear()
                self?.resizePanel(expanded: false)
            },
            onCancel: { [weak self] in self?.close() }
        ))
        resignObserver = NotificationCenter.default.addObserver(
            forName: NSWindow.didResignKeyNotification,
            object: panel,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.close() }
        }
        return panel
    }
}

/// A borderless-looking panel still has to take keyboard focus for the field.
private final class KeyPanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }
}

private struct QuickAskField: View {
    @ObservedObject var session: QuickAskSession
    let onSubmit: (String) -> Void
    let onContinueDesktop: (String) -> Void
    let onOpenDashboard: (String) -> Void
    let onClear: () -> Void
    let onCancel: () -> Void
    @State private var text = ""
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 10) {
                Image(systemName: "sparkle")
                    .foregroundStyle(.secondary)
                TextField("Ask Sjel… (stays in session)", text: $text)
                    .textFieldStyle(.plain)
                    .font(.system(size: 17))
                    .focused($focused)
                    .onSubmit {
                        let question = text.trimmingCharacters(in: .whitespacesAndNewlines)
                        guard !question.isEmpty else { return }
                        text = ""
                        onSubmit(question)
                    }
                if session.busy {
                    ProgressView().controlSize(.small)
                } else {
                    Text("↩ ask · ⌘O desktop")
                        .font(.system(size: 10))
                        .foregroundStyle(.tertiary)
                }
            }
            .padding(.horizontal, 16)
            .frame(height: 56)

            if !session.entries.isEmpty {
                Divider()
                ScrollViewReader { proxy in
                    ScrollView {
                        VStack(alignment: .leading, spacing: 10) {
                            ForEach(session.entries) { entry in
                                VStack(alignment: .leading, spacing: 3) {
                                    HStack {
                                        Text(entry.role == "user" ? "You" : entry.role == "assistant" ? "Sjel" : "Error")
                                            .font(.system(size: 11, weight: .semibold))
                                            .foregroundStyle(entry.role == "user" ? .blue : entry.role == "assistant" ? .green : .orange)
                                        Spacer()
                                        if let meta = entry.meta {
                                            Text(meta)
                                                .font(.system(size: 10, design: .monospaced))
                                                .foregroundStyle(.secondary)
                                        }
                                    }
                                    Text(entry.content)
                                        .font(.system(size: 13))
                                        .textSelection(.enabled)
                                        .fixedSize(horizontal: false, vertical: true)
                                }
                                .id(entry.id)
                            }
                            if session.busy {
                                Text("Reading capabilities and thinking…")
                                    .font(.system(size: 12))
                                    .foregroundStyle(.secondary)
                            }
                        }
                        .padding(14)
                    }
                    .onChange(of: session.entries.count) { _, _ in
                        if let last = session.entries.last?.id {
                            proxy.scrollTo(last, anchor: .bottom)
                        }
                    }
                }
                Divider()
                HStack(spacing: 10) {
                    Button("Continue in Sjel Desktop") {
                        onContinueDesktop(text.trimmingCharacters(in: .whitespacesAndNewlines))
                    }
                    .keyboardShortcut("o", modifiers: .command)
                    .controlSize(.small)

                    Button("Dashboard") {
                        onOpenDashboard(text.trimmingCharacters(in: .whitespacesAndNewlines))
                    }
                    .controlSize(.small)
                    .buttonStyle(.borderless)

                    Spacer()

                    Button("Clear", action: onClear)
                        .controlSize(.small)
                        .buttonStyle(.borderless)
                        .disabled(session.busy)
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
            }
        }
        .frame(width: 560)
        .onAppear { focused = true }
        .onExitCommand(perform: onCancel)
    }
}
