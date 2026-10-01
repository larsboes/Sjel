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
