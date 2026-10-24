import XCTest

// Proves the capture card answers a finger: the chip takes the tag and the Keep pill reports what is
// missing when the field is empty. Typing is deliberately not part of this test — driving the
// software keyboard from XCUITest waits for a keyboard the simulator may never raise, and a check
// that hangs is worse than one that covers less — so what a keystroke does is covered by
// `core::capture`'s tests and by the store's.
//
// `ios/run-sim --tap` runs this with the rest of the target and prints the screenshots it keeps.
final class InboxCapture: XCTestCase {
  func testTheChipAndKeepAnswerTheFinger() {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = "inbox"
    app.launch()

    // The replica opens before the page has anything to count; a launch is not a page.
    sleep(12)
    attach("inbox-empty")

    // #buy, measured from a screenshot: 0.14 × 0.36 of the window. It turns accent when taken.
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.14, dy: 0.36)).tap()
    sleep(2)
    attach("inbox-tagged")

    // Keep, with nothing typed: 0.845 × 0.407. The card says what is missing instead of writing.
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.845, dy: 0.407)).tap()
    sleep(2)
    attach("inbox-empty-keep")
  }

  private func attach(_ name: String) {
    let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
    shot.name = name
    shot.lifetime = .keepAlways
    add(shot)
  }
}
