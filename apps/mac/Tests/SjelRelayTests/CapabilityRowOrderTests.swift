import Foundation
import Testing
@testable import SjelRelay

@Suite struct CapabilityRowOrderTests {
    private func health(_ up: [String: Bool]) throws -> NodeHealth {
        let caps = up.map { name, isUp in #""\#(name)":{"up":\#(isUp)}"# }.joined(separator: ",")
        let json = #"{"ok":true,"version":"v1","uptime_seconds":60,"capabilities":{\#(caps)}}"#
        return try JSONDecoder().decode(NodeHealth.self, from: Data(json.utf8))
    }

    /// Starting one capability changes its dot and its button. It must not move any row.
    @Test func startingOneCapabilityMovesNoRow() throws {
        let before = try health(["travel": false, "comms": true, "calendar": true])
        let after = try health(["travel": true, "comms": true, "calendar": true])
        #expect(before.rowOrder == ["calendar", "comms", "travel"])
        #expect(after.rowOrder == before.rowOrder)
        #expect(before.down == ["travel"])
        #expect(after.down.isEmpty)
    }
}
