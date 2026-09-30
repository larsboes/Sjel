# Tauri local network plugin

Finds an Sjel node on the same Wi-Fi (PRD Q119, "Same Wi-Fi").

- `browse` runs an `NWBrowser` for `_sjel._tcp` for up to ten seconds (three by default) and
  returns each node's name, `<host>.local`, port and certificate fingerprint, read from the TXT
  record that `capabilities/sjel-status/src/lan.rs` registers.
- A result is **unverified**. Anyone on the Wi-Fi can advertise `_sjel._tcp`. The app shows the
  first 16 hex characters of the fingerprint, and the person compares them with the code the Mac
  shows under Devices before the app pins it (`connection_local_set` in
  `dashboard/src-tauri/src/mac_bridge.rs`).
- iOS asks for local-network access the first time. A refusal comes back as `denied: true`, so
  the app can say where to turn it on. The app's `Info.plist` carries
  `NSLocalNetworkUsageDescription` and `NSBonjourServices`.

Desktop and Android builds expose nothing.
