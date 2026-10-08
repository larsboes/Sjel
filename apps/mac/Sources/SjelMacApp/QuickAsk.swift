import AppKit
import Carbon.HIToolbox
import SwiftUI

/// ⌃⌥Space from any app opens one field. Enter sends the question to the dashboard's Ask
/// drawer (`/?ask=…`, read in `dashboard/src/routes/+layout.svelte`), which answers from
/// the capabilities. The answer engine stays in one place, the dashboard.
///
/// A Carbon hot key, because it needs no Accessibility permission. The shortcut is fixed:
/// ⌘Space is Spotlight, and ⌥⌘J is the browser's console.
@MainActor
final class QuickAsk {
    private var hotKey: EventHotKeyRef?
    private var handler: EventHandlerRef?
    private var panel: NSPanel?
    private let send: (String) -> Void

    init(send: @escaping (String) -> Void) {
        self.send = send
        var spec = EventTypeSpec(eventClass: OSType(kEventClassKeyboard), eventKind: UInt32(kEventHotKeyPressed))
        let me = Unmanaged.passUnretained(self).toOpaque()
        InstallEventHandler(GetApplicationEventTarget(), { _, _, context in
            guard let context else { return noErr }
            let quickAsk = Unmanaged<QuickAsk>.fromOpaque(context).takeUnretainedValue()
            // Carbon delivers hot keys on the main run loop.
            MainActor.assumeIsolated { quickAsk.show() }
            return noErr
        }, 1, &spec, me, &handler)
        let id = EventHotKeyID(signature: OSType(0x534A_454C), id: 1) // "SJEL"
        RegisterEventHotKey(UInt32(kVK_Space), UInt32(controlKey | optionKey), id, GetApplicationEventTarget(), 0, &hotKey)
    }

    func show() {
        let panel = panel ?? makePanel()
        self.panel = panel
        panel.center()
        NSApp.activate()
        panel.makeKeyAndOrderFront(nil)
    }

    private func close() {
        panel?.orderOut(nil)
    }

    private func makePanel() -> NSPanel {
        let panel = KeyPanel(
            contentRect: NSRect(x: 0, y: 0, width: 520, height: 52),
            styleMask: [.titled, .fullSizeContentView, .nonactivatingPanel],
            backing: .buffered,
            defer: true
        )
        panel.titleVisibility = .hidden
        panel.titlebarAppearsTransparent = true
        panel.isMovableByWindowBackground = true
        panel.level = .floating
        panel.hidesOnDeactivate = true
        panel.standardWindowButton(.closeButton)?.isHidden = true
        panel.standardWindowButton(.miniaturizeButton)?.isHidden = true
        panel.standardWindowButton(.zoomButton)?.isHidden = true
        panel.contentView = NSHostingView(rootView: QuickAskField(
            onSubmit: { [weak self] text in
                self?.close()
                self?.send(text)
            },
            onCancel: { [weak self] in self?.close() }
        ))
        return panel
    }
}

/// A borderless-looking panel still has to take keyboard focus for the field.
private final class KeyPanel: NSPanel {
    override var canBecomeKey: Bool { true }
}

private struct QuickAskField: View {
    let onSubmit: (String) -> Void
    let onCancel: () -> Void
    @State private var text = ""
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "sparkle")
                .foregroundStyle(.secondary)
            TextField("Ask Sjel", text: $text)
                .textFieldStyle(.plain)
                .font(.system(size: 18))
                .focused($focused)
                .onSubmit {
                    let question = text.trimmingCharacters(in: .whitespacesAndNewlines)
                    guard !question.isEmpty else { return }
                    text = ""
                    onSubmit(question)
                }
            Text("↩ opens the dashboard")
                .font(.system(size: 10))
                .foregroundStyle(.tertiary)
        }
        .padding(.horizontal, 16)
        .frame(width: 520, height: 52)
        .onAppear { focused = true }
        .onExitCommand(perform: onCancel)
    }
}
