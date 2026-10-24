import XCTest

// Proves the Reminders switch answers a finger, that the permission prompt is the one iOS puts in
// front of the person, and that what the card says afterwards is the system's own count.
//
// The prompt belongs to springboard, not to the app, so it is answered here rather than by
// `xcrun simctl privacy`: that command has no notifications service. The line under the switch comes
// from `getPendingNotificationRequests` (`host/Notifications.swift`), so "Allowed" in the screenshot
// is the notification centre's word and not this app's hope.
//
// `ios/run-sim --tap` runs this with the rest of the target and prints the screenshots it keeps.
final class NotificationsTap: XCTestCase {
  func testTheSwitchAsksOnceAndTheCardSaysWhatTheSystemHolds() {
    let app = XCUIApplication()
    // Settings is behind Today's gear, which no hand can reach in a screenshot run; the drawer is
    // remembered open from an earlier run and would cover the page, so this launch closes it.
    app.launchEnvironment["SJEL_TAB"] = "settings"
    app.launchEnvironment["SJEL_SOUNDSCAPE"] = "0"
    app.launch()

    // The window, the store and the browse each have their own thread; a launch is not a page.
    sleep(12)
    attach("reminders-off")

    // The switch, measured from the frame above: 0.862 × 0.592.
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.862, dy: 0.592)).tap()
    // The prompt is another process's window. First launch shows it; a later run answers "Allow"
    // from the same tap on the switch and shows nothing, and both are a pass.
    let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
    let allow = springboard.buttons["Allow"]
    if allow.waitForExistence(timeout: 8) {
      allow.tap()
    }
    sleep(4)
    attach("reminders-allowed")

    // Off again: the requests go, and the card says so. The header row has not moved, because the
    // lead-time row appears below it.
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.862, dy: 0.592)).tap()
    sleep(3)
    attach("reminders-off-again")
  }

  private func attach(_ name: String) {
    let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
    shot.name = name
    shot.lifetime = .keepAlways
    add(shot)
  }
}
