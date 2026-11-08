import XCTest

final class NshScreens: XCTestCase {
  func testDocumentsRenders() {
    assertScreen("documents", heading: "Documents")
  }

  func testMailRenders() {
    assertScreen("mail", heading: "Mail")
  }

  private func assertScreen(_ tab: String, heading: String) {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = tab
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launch()
    defer {
      let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
      shot.name = "nsh-\(tab)"
      shot.lifetime = .keepAlways
      add(shot)
      app.terminate()
    }

    let title = app.staticTexts[heading].firstMatch
    let visible = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "exists == true AND hittable == true"), object: title)
    XCTAssertEqual(XCTWaiter.wait(for: [visible], timeout: 15), .completed,
                   "\(heading) must render visibly; launching an empty host is not success")
  }
}
