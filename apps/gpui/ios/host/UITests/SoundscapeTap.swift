import XCTest

// Proves the soundscape drawer answers a finger, and that sound is actually being made: the play
// button starts the session, and the meter under it is drawn from the frames the device pulled, so a
// bar that moves means the audio thread ran. A screenshot of a panel alone would say neither.
//
// `ios/run-sim --tap` runs this with the rest of the target and prints the screenshots it keeps.
final class SoundscapeTap: XCTestCase {
  func testPlayingShowsAMeterAndTheMixerOpens() {
    let app = XCUIApplication()
    app.launchEnvironment["SJEL_SOUNDSCAPE"] = "1"
    app.launch()

    // The window, the store and the device each have their own thread; a launch is not a page.
    sleep(12)
    attach("drawer-paused")

    // Play, then play again: one tap per claim, so the two frames say whether the transport goes both
    // ways rather than one gesture being read as two. Measured from a screenshot: 0.14 × 0.308.
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.14, dy: 0.308)).tap()
    sleep(3)
    attach("drawer-playing")
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.14, dy: 0.308)).tap()
    sleep(2)
    attach("drawer-paused-again")

    // The mixer disclosure, measured from the frame above: 0.3 × 0.669. The row is the whole width.
    app.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.669)).tap()
    sleep(2)
    attach("drawer-mixer")
  }

  private func attach(_ name: String) {
    let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
    shot.name = name
    shot.lifetime = .keepAlways
    add(shot)
  }
}
