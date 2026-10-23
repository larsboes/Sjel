import Foundation
import Testing
@testable import SjelRelay

@Suite struct HostFindingTests {
    @Test func aFindingDecodesFromTheHostWatchAnswerAndExtraFieldsAreIgnored() throws {
        let data = Data(#"{"key":"credential:google-comms","title":"Google: Gmail and Calendar does not work","note":"token refresh failed with HTTP 400. Reconnect.","first_seen":"2026-10-23 14:00:00","status":"open"}"#.utf8)
        let finding = try JSONDecoder().decode(HostFinding.self, from: data)
        #expect(finding.title.hasSuffix("does not work"))
        #expect(finding.occurrence == "finding:credential:google-comms@2026-10-23 14:00:00")
    }

    @Test func aFindingThatClearsAndReturnsIsANewOccurrence() {
        let first = HostFinding(key: "disk", title: "", note: "", first_seen: "2026-10-23 10:00:00")
        let again = HostFinding(key: "disk", title: "", note: "", first_seen: "2026-10-23 15:00:00")
        #expect(first.occurrence != again.occurrence)
    }
}
