import Foundation
import Testing
@testable import SjelRelay

@Suite struct AgentApprovalsTests {
    let approvals = AgentApprovals(readToken: { "deployment-token" })

    @Test func theAgentViewIsReadWithTheToken() throws {
        let request = try #require(approvals.pendingRequest())
        #expect(request.httpMethod == "GET")
        #expect(request.url?.path == "/api/sjel-status/agent")
        #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer deployment-token")
    }

    @Test func aDecisionNamesItsApprovalAndOnlyAHexId() throws {
        let request = try #require(approvals.decisionRequest(id: "a1b2c3", allow: false))
        #expect(request.url?.path == "/api/sjel-status/agent/approvals/a1b2c3")
        let body = try JSONSerialization.jsonObject(with: try #require(request.httpBody)) as? [String: Bool]
        #expect(body == ["allow": false])
        #expect(approvals.decisionRequest(id: "../mode", allow: true) == nil)
        #expect(approvals.decisionRequest(id: "", allow: true) == nil)
    }

    @Test func thePendingListIsReadFromTheAgentView() {
        let data = Data(#"{"enrolled":true,"pending":[{"id":"ab","capability":"comms","method":"POST","path":"/triage/1/gmail","preview":"{}","created_at":1,"state":"pending"}],"calls":[]}"#.utf8)
        let pending = AgentApprovals.pending(fromAgentView: data)
        #expect(pending.count == 1)
        #expect(pending[0].summary == "comms: POST /triage/1/gmail")
        #expect(AgentApprovals.pending(fromAgentView: Data("{}".utf8)).isEmpty)
    }

    @Test func noTokenMeansNoRequest() {
        let none = AgentApprovals(readToken: { nil })
        #expect(none.pendingRequest() == nil)
        #expect(none.decisionRequest(id: "ab", allow: true) == nil)
    }
}

@Suite struct CachedTokenTests {
    final class Reads: @unchecked Sendable {
        var count = 0
        var answer: DashboardLogin.KeychainRead
        init(_ answer: DashboardLogin.KeychainRead) { self.answer = answer }
    }

    @Test func aTokenIsReadOnceAndKept() {
        let reads = Reads(.token("t"))
        let cache = CachedToken(read: { reads.count += 1; return reads.answer })
        #expect(cache.get() == "t")
        #expect(cache.get() == "t")
        #expect(reads.count == 1)
    }

    @Test func aRefusalIsNotAskedAgainUntilForgotten() {
        let reads = Reads(.refused)
        let cache = CachedToken(read: { reads.count += 1; return reads.answer })
        #expect(cache.get() == nil)
        #expect(cache.get() == nil)
        #expect(reads.count == 1)
        reads.answer = .token("t")
        cache.forget()
        #expect(cache.get() == "t")
        #expect(reads.count == 2)
    }

    @Test func aMissingItemIsReadAgain() {
        let reads = Reads(.missing)
        let cache = CachedToken(read: { reads.count += 1; return reads.answer })
        #expect(cache.get() == nil)
        reads.answer = .token("t")
        #expect(cache.get() == "t")
        #expect(reads.count == 2)
    }
}

@Suite struct NodeHealthTests {
    @Test func theShellsHealthSaysWhichCapabilitiesAreDown() throws {
        let data = Data(#"{"ok":false,"version":"0.1.0","uptime_seconds":2816,"capabilities":{"transit":{"up":false,"url":"u"},"finance":{"up":true,"url":"u"},"dashboard":{"up":false,"url":"u"}}}"#.utf8)
        let health = try JSONDecoder().decode(NodeHealth.self, from: data)
        #expect(health.version == "0.1.0")
        #expect(health.uptimeSeconds == 2816)
        #expect(health.down == ["dashboard", "transit"])
    }
}
