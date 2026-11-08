import XCTest
import UIKit

final class NshScreens: XCTestCase {
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

    // GPUI paints its five tabs in Metal; Mail is second, above the home indicator.
    let mail = app.coordinate(withNormalizedOffset: .zero).withOffset(
      CGVector(dx: app.frame.width * 0.3, dy: app.frame.height - 62))
    attach("today-before-nsh")
    mail.tap()
    assertHeading(app, "Mail")
    app.buttons["Documents"].firstMatch.tap()
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
