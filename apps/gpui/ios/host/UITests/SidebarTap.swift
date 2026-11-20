import XCTest

final class SidebarTap: XCTestCase {
  func testThePanelSwitchesSeatsAndReturnsToToday() {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launchEnvironment["SJEL_GPUI_NO_AUDIO"] = "1"
    app.launch()
    defer { app.terminate() }

    let today = app.descendants(matching: .any)["today-gpui"].firstMatch
    guard today.waitForExistence(timeout: 15) else {
      XCTFail("The GPUI Today surface did not appear")
      return
    }
    today.coordinate(withNormalizedOffset: CGVector(dx: 5.5 / 7, dy: 1))
      .withOffset(CGVector(dx: 0, dy: -22)).tap()
    let agent = app.descendants(matching: .any)["agent-gpui"].firstMatch
    guard agent.waitForExistence(timeout: 10) else {
      XCTFail("The GPUI Agent panel did not open")
      return
    }
    tapPanel(2, on: agent)
    let activity = app.descendants(matching: .any)["activity-gpui"].firstMatch
    guard activity.waitForExistence(timeout: 10) else {
      XCTFail("The GPUI Activity panel did not open")
      return
    }
    tapPanel(1, on: activity)
    let sound = app.descendants(matching: .any)["soundscape-gpui"].firstMatch
    guard sound.waitForExistence(timeout: 10) else {
      XCTFail("The GPUI Soundscape panel did not open")
      return
    }
    tapPanel(5, on: sound)
    XCTAssertTrue(today.waitForExistence(timeout: 10))
  }

  private func tapPanel(_ index: CGFloat, on surface: XCUIElement) {
    surface.coordinate(withNormalizedOffset: CGVector(dx: (index + 0.5) / 6, dy: 0))
      .withOffset(CGVector(dx: 0, dy: 22)).tap()
  }
}
