import XCTest

final class InboxCapture: XCTestCase {
  func testTheChipAndKeepAnswerTheFinger() {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = "inbox"
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launch()
    defer { app.terminate() }

    let capture = app.descendants(matching: .any)["capture-gpui"].firstMatch
    guard capture.waitForExistence(timeout: 15) else {
      XCTFail("The GPUI Capture surface did not appear")
      return
    }

    let initial = capture.screenshot()
    add(XCTAttachment(screenshot: initial))

    let tag = capture.coordinate(withNormalizedOffset: CGVector(dx: 0, dy: 0))
      .withOffset(CGVector(dx: 38, dy: 110))
    tag.tap()
    let tagged = capture.screenshot()
    add(XCTAttachment(screenshot: tagged))
    XCTAssertNotEqual(tagged.pngRepresentation, initial.pngRepresentation)

    let keep = capture.coordinate(withNormalizedOffset: CGVector(dx: 1, dy: 0))
      .withOffset(CGVector(dx: -42, dy: 66))
    keep.tap()
    let rejected = capture.screenshot()
    add(XCTAttachment(screenshot: rejected))
    XCTAssertNotEqual(rejected.pngRepresentation, tagged.pngRepresentation)
  }
}
