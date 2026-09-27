//! Tauri bridge for finding an Axon node on the local network (PRD Q119).
//!
//! The iOS target browses Bonjour for `_axon._tcp` (`libs/sjel-server/src/lan.rs`) and returns
//! each node's host, port and certificate fingerprint from its TXT record. The WebView calls it
//! as `plugin:local-network|browse`; this crate only registers it. What is found is unverified
//! until the person compares the fingerprint with the one the Mac shows.

#![cfg(mobile)]

use tauri::{plugin::Builder, plugin::TauriPlugin, Runtime};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_local_network);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("local-network")
        .setup(|_app, _api| {
            #[cfg(target_os = "ios")]
            _api.register_ios_plugin(init_plugin_local_network)?;
            #[cfg(target_os = "android")]
            compile_error!("Bonjour discovery is written for iOS; do not add an Android wrapper.");
            Ok(())
        })
        .build()
}
