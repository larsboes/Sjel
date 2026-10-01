import Foundation
import Testing
@testable import SjelRelay

@Suite struct DashboardLoginTests {
    let login = DashboardLogin(readToken: { "deployment-token" })

    @Test func theTicketRequestCarriesTheTokenAsABearer() throws {
        let request = try #require(login.ticketRequest())
        #expect(request.httpMethod == "POST")
        #expect(request.url?.absoluteString == "http://127.0.0.1:8082/api/sjel-status/session/ticket")
        #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer deployment-token")
    }

    @Test func noTokenMeansNoRequest() {
        #expect(DashboardLogin(readToken: { nil }).ticketRequest() == nil)
    }

    @Test func theOpenURLIsTheShellsOwnSessionPath() throws {
        let body = Data(#"{"open":"/session/open?ticket=abc","expires_in":60}"#.utf8)
        let url = try #require(login.openURL(fromTicketResponse: body))
        #expect(url.absoluteString == "http://127.0.0.1:8082/session/open?ticket=abc")
    }

    @Test func anAnswerPointingElsewhereIsRefused() {
        for open in [
            "https://evil.example/session/open?ticket=abc",
            "//evil.example/session/open?ticket=abc",
            "/dashboard?ticket=abc",
            "/session/openx?ticket=abc",
        ] {
            let body = try! JSONSerialization.data(withJSONObject: ["open": open])
            #expect(login.openURL(fromTicketResponse: body) == nil, "\(open) must be refused")
        }
    }
}
