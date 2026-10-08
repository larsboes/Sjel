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

@Suite struct AgentViewTests {
    @Test func callsAreReadNewestFirstAndReadsAreMarked() {
        let data = Data(#"{"pending":[],"calls":[{"at":20,"capability":"comms","method":"POST","path":"/triage","status":200,"decision":"allowed"},{"at":10,"capability":"vault","method":"GET","path":"/routes","status":200,"decision":"read"}]}"#.utf8)
        let view = AgentView.decode(data)
        #expect(view.calls.count == 2)
        #expect(view.calls[0].isRead == false)
        #expect(view.calls[1].isRead)
    }
}

@Suite struct LifecycleRequestTests {
    let bridge = NodeBridge(token: CachedToken(read: { .token("t") }))

    @Test func aCapabilityNameBecomesOnePathSegment() throws {
        let request = try #require(bridge.lifecycleRequest("knowledge-graph", false))
        #expect(request.httpMethod == "POST")
        #expect(request.url?.path == "/api/sjel-status/capabilities/knowledge-graph/stop")
        #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer t")
    }

    @Test func aNameThatCouldBuildAPathIsRefused() {
        #expect(bridge.lifecycleRequest("../agent/mode", true) == nil)
        #expect(bridge.lifecycleRequest("", true) == nil)
    }
}

@Suite struct TodayTests {
    func entry(_ id: String, _ start: String, _ end: String, allDay: Bool) -> CalendarEntry {
        let json = #"{"id":"\#(id)","title":"\#(id)","starts_at":"\#(start)","ends_at":"\#(end)","all_day":\#(allDay),"commitment":"planned"}"#
        return try! JSONDecoder().decode(CalendarEntry.self, from: Data(json.utf8))
    }

    let now = Today.time.date(from: "2026-10-08T09:30:00")!

    @Test func finishedEntriesDropAndAllDayLeads() {
        let entries = [
            entry("standup", "2026-10-08T09:00:00", "2026-10-08T09:15:00", allDay: false),
            entry("lunch", "2026-10-08T12:00:00", "2026-10-08T13:00:00", allDay: false),
            entry("berlin", "2026-10-07", "2026-10-14", allDay: true),
        ]
        #expect(Today.ahead(entries, now: now).map(\.id) == ["berlin", "lunch"])
    }

    @Test func aMultiDayEntryNamesItsLastDay() {
        let berlin = entry("berlin", "2026-10-07", "2026-10-14", allDay: true)
        #expect(Today.when(berlin, now: now).hasPrefix("until "))
        #expect(Today.when(berlin, now: now).contains("13"))
    }

    @Test func aTimedEntryOnTheNextDaySaysTomorrow() {
        let train = entry("train", "2026-10-09T07:10:00", "2026-10-09T12:00:00", allDay: false)
        #expect(Today.when(train, now: now).hasPrefix("tomorrow "))
    }

    @Test func theRangeIsTodayAndTomorrow() {
        let range = Today.range(now: now)
        #expect(range.from == "2026-10-08")
        #expect(range.to == "2026-10-09")
    }
}
