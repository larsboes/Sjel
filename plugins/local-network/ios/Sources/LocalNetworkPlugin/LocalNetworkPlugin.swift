import Foundation
import Network
import Tauri
import UIKit
import WebKit

// Finds Axon nodes on the local network (PRD Q119). The Mac advertises `_axon._tcp` with its host,
// port and certificate fingerprint in the TXT record (capabilities/sjel-status/src/lan.rs), so a
// browse result is enough to connect: no service resolution, no connection from here.
//
// The first browse shows iOS's local-network permission prompt, which needs
// NSLocalNetworkUsageDescription and NSBonjourServices in the app's Info.plist. A refusal is
// reported as `denied`, so the app can say where to turn it on instead of finding nothing.

private struct BrowseArguments: Decodable {
  let timeoutMs: Int?
}

private struct FoundNode: Encodable {
  let name: String
  let host: String
  let port: Int
  let fingerprint: String
}

private struct BrowsePayload: Encodable {
  let nodes: [FoundNode]
  /// True when iOS refused local-network access for this app.
  let denied: Bool
}

final class LocalNetworkPlugin: Plugin {
  private var browser: NWBrowser?

  @objc public func browse(_ invoke: Invoke) {
    let args = try? invoke.parseArgs(BrowseArguments.self)
    let timeout = Double(min(max(args?.timeoutMs ?? 3000, 500), 10_000)) / 1000

    browser?.cancel()
    let browser = NWBrowser(
      for: .bonjourWithTXTRecord(type: "_axon._tcp", domain: nil), using: .tcp)
    self.browser = browser

    // Every handler runs on the main queue, so these need no lock.
    var found: [String: FoundNode] = [:]
    var denied = false

    browser.browseResultsChangedHandler = { results, _ in
      for result in results {
        guard case let .service(name, _, _, _) = result.endpoint,
          case let .bonjour(record) = result.metadata
        else { continue }
        let txt = record.dictionary
        guard let host = txt["host"], !host.isEmpty,
          let port = txt["port"].flatMap(Int.init), (1...65_535).contains(port),
          let fingerprint = txt["fp"], fingerprint.count == 64
        else { continue }
        found[name] = FoundNode(
          name: name, host: "\(host).local", port: port, fingerprint: fingerprint.lowercased())
      }
    }
    browser.stateUpdateHandler = { state in
      // iOS reports a refused local-network permission as a DNS-SD policy error.
      if case let .waiting(error) = state, case let .dns(code) = error,
        code == DNSServiceErrorType(kDNSServiceErr_PolicyDenied)
      {
        denied = true
      }
    }
    browser.start(queue: .main)

    DispatchQueue.main.asyncAfter(deadline: .now() + timeout) { [weak self] in
      browser.cancel()
      if self?.browser === browser { self?.browser = nil }
      let nodes = found.values.sorted { $0.name < $1.name }
      invoke.resolve(BrowsePayload(nodes: nodes, denied: denied))
    }
  }
}

@_cdecl("init_plugin_local_network")
func initPlugin() -> Plugin {
  return LocalNetworkPlugin()
}
