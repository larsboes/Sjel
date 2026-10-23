import XCTest

// Proves the camera entry point is reachable by a finger: Settings → Scan opens the scanner that
// PairScanner.swift presents. The simulator has no camera, so what appears is the scanner's own
// "no camera" state — which is exactly the path a phone without one takes, and the reason the
// message points at Join.
//
// The coordinates are measured from screenshots because GPUI paints its own pixels: nothing on
// screen is a UIKit view, so there is no element to query by identifier.
//
// ios/run-sim --tap runs this and prints the screenshots it keeps.
final class ScanTap: XCTestCase {
  func testTappingScanOpensTheCameraScreen() {
    let app = XCUIApplication()
    app.launch()
    sleep(6)

    // The Settings tab, bottom right: 0.81 × 0.93 of the window.
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.81, dy: 0.934)).tap()
    sleep(3)
    attach("settings")

    // The Scan pill on the "Scan your Mac's code" card, measured from the `settings` shot above:
    // 0.835 × 0.306 of the window.
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.835, dy: 0.306)).tap()
    sleep(3)
    attach("after-scan-tap")

    // The scanner is a presented UIKit view controller, so its Close button is a real element:
    // its existence is the check, independent of what the camera answered.
    XCTAssertTrue(
      app.buttons["Close"].waitForExistence(timeout: 5),
      "tapping Scan must present the scanner"
    )
  }

  private func attach(_ name: String) {
    let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
    shot.name = name
    shot.lifetime = .keepAlways
    add(shot)
  }
}
