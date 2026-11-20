import XCTest
import UIKit

final class GpuiScreens: XCTestCase {
  func testMapRendersAndReturnsToTrips() {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = "map"
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launch()
    defer { attach("gpui-map"); app.terminate() }
    let map = assertGPUI(app, route: "map")
    guard map.exists else { return }
    map.coordinate(withNormalizedOffset: CGVector(dx: 0.09, dy: 0))
      .withOffset(CGVector(dx: 0, dy: 28)).tap()
    assertGPUI(app, route: "travel")
  }

  func testProjectsRenders() {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = "projects"
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launch()
    defer { attach("gpui-projects"); app.terminate() }
    assertGPUI(app, route: "projects")
  }
  func testPeopleAndPlacesRenders() {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = "people"
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launch()
    defer { attach("gpui-people"); app.terminate() }
    assertGPUI(app, route: "people")
  }
  func testInventoryAndWishlistRenders() {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = "inventory"
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launch()
    defer { attach("gpui-inventory"); app.terminate() }
    assertGPUI(app, route: "inventory")
  }
  func testFinanceRenders() {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = "finance"
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launch()
    defer { attach("gpui-finance"); app.terminate() }
    assertGPUI(app, route: "finance")
  }
  func testCalendarRenders() {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = "calendar"
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launch()
    defer { attach("gpui-calendar"); app.terminate() }
    assertGPUI(app, route: "calendar")
  }
  func testSettingsRenders() {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = "settings"
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launch()
    defer { attach("gpui-settings"); app.terminate() }
    assertGPUI(app, route: "settings")
  }
  func testInteriorRenders() { assertScreen("interior") }
  func testTripsRenders() { assertScreen("travel") }
  func testAgentRenders() { assertSeat("agent") }
  func testTerminalRenders() { assertSeat("terminal") }
  func testSoundscapeRenders() { assertSeat("soundscape") }
  func testActivityRenders() { assertSeat("activity") }
  func testScratchpadRenders() { assertSeat("scratchpad") }

  private func assertSeat(_ seat: String) {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = "today"
    app.launchEnvironment["SJEL_SIDEBAR"] = seat
    app.launchEnvironment["SJEL_GPUI_NO_AUDIO"] = "1"
    app.launch()
    defer { attach("gpui-\(seat)"); app.terminate() }
    let panel = assertGPUI(app, route: seat)
    guard panel.exists else { return }
    panel.coordinate(withNormalizedOffset: CGVector(dx: 5.5 / 6, dy: 0))
      .withOffset(CGVector(dx: 0, dy: 22)).tap()
    assertGPUI(app, route: "today")
  }

  func testDocumentsRenders() {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = "documents"
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launch()
    defer { attach("gpui-documents"); app.terminate() }
    assertGPUI(app, route: "documents")
  }

  func testMailRenders() {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = "mail"
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launch()
    defer { attach("gpui-mail"); app.terminate() }
    assertGPUI(app, route: "mail")
  }

  func testNavigationBetweenGpuiRoutes() {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = "today"
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launch()
    defer { attach("gpui-route-navigation"); app.terminate() }

    let today = assertGPUI(app, route: "today")
    guard today.exists else { return }
    tapTab(1, on: today)
    let mail = assertGPUI(app, route: "mail")
    guard mail.exists else { return }
    tapTab(2, on: mail)
    let documents = assertGPUI(app, route: "documents")
    guard documents.exists else { return }
    documents.coordinate(withNormalizedOffset: CGVector(dx: 0.08, dy: 0.04)).tap()
    assertGPUI(app, route: "mail")
  }

  func testDeepLinkOpensGpuiFinance() throws {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.open(try XCTUnwrap(URL(string: "sjel://finance")))
    defer { app.terminate() }
    assertGPUI(app, route: "finance")
  }

  @discardableResult
  private func assertGPUI(_ app: XCUIApplication, route: String,
                          file: StaticString = #filePath, line: UInt = #line) -> XCUIElement {
    let surface = app.descendants(matching: .any)["\(route)-gpui"].firstMatch
    XCTAssertTrue(surface.waitForExistence(timeout: 15), file: file, line: line)
    XCTAssertFalse(app.staticTexts["Sjel could not start."].exists, file: file, line: line)
    if surface.exists {
      assertTextContrast(surface)
    }
    return surface
  }

  private func tapTab(_ index: CGFloat, on surface: XCUIElement) {
    surface.coordinate(withNormalizedOffset: CGVector(dx: (index + 0.5) / 7, dy: 1))
      .withOffset(CGVector(dx: 0, dy: -22)).tap()
  }

  private func assertScreen(_ tab: String) {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = tab
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launch()
    defer { attach("gpui-\(tab)"); app.terminate() }
    assertGPUI(app, route: tab)
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
