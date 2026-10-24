import XCTest

// Proves the right edge answers a finger: the list button in Today's header opens it on the activity
// seat, the chip beside the title moves it to the soundscape, and the ✕ puts it away.
//
// One panel with seats is the point (`ui/src/sidebar.rs`): the soundscape used to own this edge, and
// two panels cannot. The screenshots are the claim — the title and the lit chip are the frame's, so a
// frame that drew the wrong seat would be visible here.
//
// `ios/run-sim --tap` runs this with the rest of the target and prints the screenshots it keeps.
final class SidebarTap: XCTestCase {
  func testTheEdgeOpensOnActivityAndMovesToSound() {
    let app = XCUIApplication()
    // The edge is remembered open, so a run that needs to see it closed says so for this launch.
    app.launchEnvironment["SJEL_SIDEBAR"] = "off"
    app.launch()

    // The window, the store and the browse each have their own thread; a launch is not a page.
    sleep(12)
    attach("edge-closed")

    // The list button in Today's header, measured from the frame above: 0.808 × 0.233.
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.808, dy: 0.233)).tap()
    sleep(3)
    attach("edge-activity")

    // The Sound chip, in the panel's own header row at the same height: 0.512 × 0.233.
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.512, dy: 0.233)).tap()
    sleep(3)
    attach("edge-sound")

    // The ✕, right of the chips: 0.917 × 0.233.
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.917, dy: 0.233)).tap()
    sleep(3)
    attach("edge-away")
  }

  private func attach(_ name: String) {
    let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
    shot.name = name
    shot.lifetime = .keepAlways
    add(shot)
  }
}
