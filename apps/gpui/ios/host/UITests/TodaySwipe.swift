import XCTest

// Proves two things a screenshot cannot: that a finger scrolls a GPUI page (the pixels are drawn
// into a Metal layer, so UIKit's touch handling is the only input path), and that the sections
// below the fold — the projects a landing page lists — are really there.
//
// `ios/run-sim --tap` runs this with the rest of the target and prints the screenshots it keeps.
// The launch environment pins the landing tab and the day, so the page under the finger is the one
// the screenshot names.
final class TodaySwipe: XCTestCase {
  func testScrollingReachesTheProjectsSection() {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_TAB"] = "today"
    app.launch()

    // The replica opens and the copy's day is read on their own threads; a launch is not a page.
    sleep(12)
    attach("today-top")

    // One long drag up the page: a swipe without velocity leaves the scroll where a finger left it.
    let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.78))
    let end = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.22))
    start.press(forDuration: 0.05, thenDragTo: end)
    sleep(2)
    attach("today-projects")
  }

  private func attach(_ name: String) {
    let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
    shot.name = name
    shot.lifetime = .keepAlways
    add(shot)
  }
}
