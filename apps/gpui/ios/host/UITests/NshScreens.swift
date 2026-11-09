import XCTest
import UIKit

final class NshScreens: XCTestCase {
  func testMapRendersAndReturnsToTrips() {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = "map"
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launch()
    defer { attach("nsh-map"); app.terminate() }
    assertHeading(app, "Map & Places")
    let zoom = app.buttons["Zoom in"].firstMatch
    XCTAssertTrue(zoom.waitForExistence(timeout: 10))
    zoom.tap()
    app.buttons["Online tiles"].firstMatch.tap()
    XCTAssertTrue(app.buttons["Cached only"].firstMatch.waitForExistence(timeout: 5))
    app.buttons["Trips"].firstMatch.tap()
    XCTAssertFalse(app.buttons["Zoom in"].firstMatch.waitForExistence(timeout: 3))
  }

  func testProjectsRenders() { assertScreen("projects", heading: "Projects") }
  func testInteriorRenders() { assertScreen("interior", heading: "Interior") }
  func testTripsRenders() { assertScreen("travel", heading: "Trips") }
  func testAgentRenders() { assertSeat("agent", heading: "Agent") }
  func testTerminalStartsLocked() { assertSeat("terminal", heading: "Terminal locked") }
  func testSoundscapeRenders() { assertSeat("soundscape", heading: "Soundscape") }
  func testActivityRenders() { assertSeat("activity", heading: "Activity") }

  private func assertSeat(_ seat: String, heading: String) {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = "today"
    app.launchEnvironment["SJEL_SIDEBAR"] = seat
    app.launchEnvironment["SJEL_GPUI_NO_AUDIO"] = "1"
    app.launch()
    defer { attach("nsh-\(seat)"); app.terminate() }
    assertHeading(app, heading)
    app.buttons["Close panel"].firstMatch.tap()
    assertHeading(app, "TODAY")
  }

  func testDocumentsRenders() {
    assertScreen("documents", heading: "Documents")
  }

  func testMailRenders() {
    assertScreen("mail", heading: "Mail")
  }

  func testOpeningNshAfterTodayRenders() {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = "today"
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launch()
    defer { attach("nsh-after-today"); app.terminate() }

    let mail = app.buttons["Mail"].firstMatch
    attach("today-before-nsh")
    mail.tap()
    assertHeading(app, "Mail")
    app.buttons["Docs"].firstMatch.tap()
    assertHeading(app, "Documents")
  }

  private func assertScreen(_ tab: String, heading: String) {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = tab
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launch()
    defer {
      attach("nsh-\(tab)")
      app.terminate()
    }
    assertHeading(app, heading)
  }

  private func assertHeading(_ app: XCUIApplication, _ heading: String) {
    let title = app.staticTexts[heading].firstMatch
    let visible = XCTNSPredicateExpectation(
      predicate: NSPredicate(format: "exists == true AND hittable == true"), object: title)
    guard XCTWaiter.wait(for: [visible], timeout: 15) == .completed else {
      XCTFail("\(heading) must render visibly; launching an empty host is not success")
      return
    }
    assertTextContrast(title)
  }

  private func attach(_ name: String) {
    let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
    shot.name = name
    shot.lifetime = .keepAlways
    add(shot)
  }

  private func assertTextContrast(_ title: XCUIElement) {
    guard let image = title.screenshot().image.cgImage,
          let pixels = CGContext(data: nil, width: image.width, height: image.height,
                                 bitsPerComponent: 8, bytesPerRow: image.width * 4,
                                 space: CGColorSpaceCreateDeviceRGB(),
                                 bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue),
          let data = pixels.data else {
      XCTFail("The heading screenshot must contain readable pixels")
      return
    }
    pixels.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
    let bytes = data.assumingMemoryBound(to: UInt8.self)
    var darkest = 255
    var brightest = 0
    for offset in stride(from: 0, to: image.width * image.height * 4, by: 4) {
      let brightness = (Int(bytes[offset]) + Int(bytes[offset + 1]) + Int(bytes[offset + 2])) / 3
      darkest = min(darkest, brightness)
      brightest = max(brightest, brightness)
    }
    XCTAssertGreaterThan(brightest - darkest, 40,
                         "The heading must paint contrasting text, not an empty black rectangle")
  }
}
