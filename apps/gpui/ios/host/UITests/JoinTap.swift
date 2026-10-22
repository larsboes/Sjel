import XCTest

// Proves a finger reaches GPUI, which no screenshot can: every pixel on screen is drawn by GPUI
// into a Metal layer, so UIKit's touch handling is the only input path (gpui-pre-mobile's
// src/ios/window.rs maps touchesBegan to GPUI's touch recognizer, which synthesizes the tap's
// mouse down and up). A tap that turns the card into "Asking … to allow this device" therefore
// covers UIKit → touch → the view's `on_mouse_down` handler, and the join request that follows.
//
// ios/run-sim --tap runs this and prints the two screenshots it keeps.
final class JoinTap: XCTestCase {
  func testTappingJoinAsksTheMac() {
    let app = XCUIApplication()
    app.launch()

    // The card appears when a browse answer arrives; Bonjour takes a moment after launch.
    sleep(10)
    attach("before-tap")

    // The Join pill's centre, measured from a screenshot: 0.842 × 0.294 of the window. GPUI takes
    // touches in its own view, which UIKit places below the navigation bar, so a tap here is 85 px
    // inside the view's 706 px height.
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.842, dy: 0.294)).tap()
    sleep(4)
    attach("after-tap")
  }

  private func attach(_ name: String) {
    let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
    shot.name = name
    shot.lifetime = .keepAlways
    add(shot)
  }
}
