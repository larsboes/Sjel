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
    // Settings is behind Today's gear since the bar became five tabs, so a run that needs the page
    // asks for it rather than hunting for a tab that is no longer there. The right edge is also
    // remembered open and would cover the page (`ui/src/sidebar.rs`), so this launch closes it.
    app.launchEnvironment["SJEL_TAB"] = "settings"
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launch()
    sleep(6)
    attach("settings")

    // The Scan pill on the "Scan your Mac's code" card, measured from the `settings` shot above:
    // 0.832 × 0.367 of the window.
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.832, dy: 0.367)).tap()
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
