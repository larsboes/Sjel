import AVFoundation
import UIKit

// The camera the Mac's Setup page expects a phone to point at its pairing code.
//
// The code carries the Mac's address, the SHA-256 of its listener certificate and a one-time
// code. Reading it with the camera is what makes the pairing trustworthy: the pin comes from the
// screen, not from whoever answered on the network, so the claim can be pinned to exactly one
// certificate. Rust decides what the text means (`sjel_ios_scanned_pairing`, src/lib.rs); this
// file only reads it and says so.
//
// Swift owns the camera preview because a preview is an `AVCaptureVideoPreviewLayer`. GPUI draws
// its own pixels and has nowhere to put one, so the scanner is a presented view controller, the
// app's first. Nothing here keeps a frame: the session stops the moment a code is read, and again
// on dismissal, so the camera is never on while the person is looking at something else.

/// The text this screen shows when the camera cannot open. The Join row on the settings page stays
/// the way in for a phone without one, so the sentence says where that is.
private let noCamera = """
No camera to scan with. Allow the camera in Settings, or tap Join on your Mac instead.
"""

private final class PairScannerController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    private let session = AVCaptureSession()
    private var preview: AVCaptureVideoPreviewLayer?
    private var status: UILabel?
    /// Set once a code is handed to Rust: a second frame of the same code must not claim twice.
    private var finished = false

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        title = "Scan the code"

        let close = UIBarButtonItem(title: "Close", style: .done, target: self, action: #selector(dismissSelf))
        navigationItem.rightBarButtonItem = close

        let label = UILabel()
        label.text = "Point the camera at the code on your Mac’s Setup page."
        label.textColor = .white
        label.font = .preferredFont(forTextStyle: .footnote)
        label.numberOfLines = 0
        label.textAlignment = .center
        label.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(label)
        status = label
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 24),
            label.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -24),
            label.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -24),
        ])
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        start()
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        session.stopRunning()
    }

    /// Asks for the camera, wires the session, and starts it. Every failure ends in the same place:
    /// a sentence on screen and a message to Rust, so the page behind this one can say why nothing
    /// happened instead of leaving the person looking at a black view controller.
    private func start() {
        guard !finished else { return }
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            configure()
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
                DispatchQueue.main.async {
                    guard let self else { return }
                    if granted {
                        self.configure()
                    } else {
                        self.fail(noCamera)
                    }
                }
            }
        default:
            fail(noCamera)
        }
    }

    private func configure() {
        guard let device = AVCaptureDevice.default(for: .video) else {
            fail(noCamera)
            return
        }
        do {
            let input = try AVCaptureDeviceInput(device: device)
            guard session.canAddInput(input) else {
                fail(noCamera)
                return
            }
            session.addInput(input)

            let output = AVCaptureMetadataOutput()
            guard session.canAddOutput(output) else {
                fail(noCamera)
                return
            }
            session.addOutput(output)
            output.setMetadataObjectsDelegate(self, queue: .main)
            // Assigned after the output has a connection: before that, `availableMetadataObjectTypes`
            // is empty and a QR-only filter would be rejected as unsupported.
            output.metadataObjectTypes = output.availableMetadataObjectTypes.contains(.qr) ? [.qr] : []

            let preview = AVCaptureVideoPreviewLayer(session: session)
            preview.frame = view.bounds
            preview.videoGravity = .resizeAspectFill
            view.layer.insertSublayer(preview, at: 0)
            self.preview = preview

            DispatchQueue.global(qos: .userInitiated).async { [session] in
                session.startRunning()
            }
        } catch {
            fail("The camera could not be opened: \(error.localizedDescription)")
        }
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        preview?.frame = view.bounds
    }

    func metadataOutput(
        _ output: AVCaptureMetadataOutput,
        didOutput metadataObjects: [AVMetadataObject],
        from connection: AVCaptureConnection
    ) {
        guard !finished,
              let code = metadataObjects
                  .compactMap({ $0 as? AVMetadataMachineReadableCodeObject })
                  .first(where: { $0.type == .qr })?
                  .stringValue,
              !code.isEmpty
        else { return }

        finished = true
        session.stopRunning()
        // The one hand-off: the string, straight to Rust. Nothing is parsed or trimmed here,
        // because what a code means belongs to `device::parse_pairing_qr` and nowhere else.
        code.withCString { sjel_ios_scanned_pairing($0) }
        dismiss(animated: true)
    }

    private func fail(_ text: String) {
        status?.text = text
        text.withCString { sjel_ios_scan_failed($0) }
    }

    @objc private func dismissSelf() {
        session.stopRunning()
        dismiss(animated: true)
    }
}

/// Opens the scanner over whatever the app is showing. Called by Rust (`present_pair_scanner` in
/// src/lib.rs) when the person taps Scan; the answer comes back through
/// `sjel_ios_scanned_pairing`, so this file never decides what a code is.
@_cdecl("sjel_ios_present_pair_scanner")
public func sjelIosPresentPairScanner() {
    DispatchQueue.main.async {
        guard let root = UIApplication.shared.connectedScenes
            .compactMap({ $0 as? UIWindowScene })
            .compactMap({ $0.keyWindow ?? $0.windows.first })
            .first?.rootViewController
        else { return }
        // Present from the top of whatever is already showing: a second presentation on a view
        // controller that is presenting fails silently, and this button can be pressed twice.
        var top = root
        while let presented = top.presentedViewController {
            top = presented
        }
        let scanner = UINavigationController(rootViewController: PairScannerController())
        scanner.modalPresentationStyle = .fullScreen
        top.present(scanner, animated: true)
    }
}
