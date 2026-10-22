import Foundation
import Network

// Finds Sjel nodes on the local network (PRD Q119). The Mac advertises `_sjel._tcp` with its host,
// port and certificate fingerprint in the TXT record (capabilities/sjel-status/src/lan.rs), so a
// browse result is enough to connect: no service resolution, no connection from here.
//
// Ported from plugins/local-network, which does the same browse for the Tauri app. Every result
// goes to Rust through `sjel_ios_nodes`, the entry point ios/src/lib.rs documents; the view there
// shows what was found and asks to join when the person taps.
//
// The first browse shows iOS's local-network permission prompt, which needs
// NSLocalNetworkUsageDescription and NSBonjourServices in the app's Info.plist. A refusal is
// reported as `denied`, so the app says where to turn it on instead of finding nothing.

final class Discovery {
  private var browser: NWBrowser?
  /// The last answer, so a repeat publish does not flicker the view.
  private var last: String = ""

  func start() {
    browser?.cancel()
    let browser = NWBrowser(
      for: .bonjourWithTXTRecord(type: "_sjel._tcp", domain: nil), using: .tcp)
    self.browser = browser

    // Every handler runs on the main queue, so these need no lock.
    browser.browseResultsChangedHandler = { [weak self] results, _ in
      var nodes: [[String: Any]] = []
      for result in results {
        guard case let .service(name, _, _, _) = result.endpoint,
          case let .bonjour(record) = result.metadata
        else { continue }
        let txt = record.dictionary
        guard let host = txt["host"], !host.isEmpty,
          let port = txt["port"].flatMap(Int.init), (1...65_535).contains(port),
          let fingerprint = txt["fp"], fingerprint.count == 64
        else { continue }
        nodes.append([
          "name": name, "host": host, "port": port, "fp": fingerprint.lowercased(),
        ])
      }
      nodes.sort { ($0["name"] as? String ?? "") < ($1["name"] as? String ?? "") }
      self?.publish(nodes: nodes, denied: false)
    }
    browser.stateUpdateHandler = { [weak self] state in
      // iOS reports a refused local-network permission as a DNS-SD policy error.
      if case let .waiting(error) = state, case let .dns(code) = error,
        code == DNSServiceErrorType(kDNSServiceErr_PolicyDenied)
      {
        self?.publish(nodes: [], denied: true)
      }
    }
    browser.start(queue: .main)
  }

  func stop() {
    browser?.cancel()
    browser = nil
  }

  private func publish(nodes: [[String: Any]], denied: Bool) {
    let payload: [String: Any] = ["nodes": nodes, "denied": denied]
    guard let data = try? JSONSerialization.data(withJSONObject: payload),
      let text = String(data: data, encoding: .utf8)
    else { return }
    guard text != last else { return }
    last = text
    text.withCString { sjel_ios_nodes($0) }
  }
}
