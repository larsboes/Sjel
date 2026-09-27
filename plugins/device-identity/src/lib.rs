//! Tauri bridge for the native device identity used by Axon's pairing protocol.
//!
//! On iOS, the native implementation creates or loads one Curve25519 signing key from the
//! platform Keychain. Only its public key crosses this boundary. Desktop builds intentionally
//! expose no software-key fallback.

#![cfg(mobile)]

use serde::{Deserialize, Serialize};
use tauri::{plugin::Builder, plugin::PluginHandle, plugin::TauriPlugin, Manager, Runtime};

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_device_identity);

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Identity {
    pub id: String,
    pub platform: String,
    pub algorithm: String,
    pub public_key: String,
}

#[derive(Debug, Serialize)]
struct SignRequest {
    message: String,
}

#[derive(Debug, Deserialize)]
struct SignResponse {
    signature: String,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("native device identity command failed: {0}")]
    Native(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Handle for the device identity native plugin.
pub struct DeviceIdentity<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> DeviceIdentity<R> {
    pub fn get(&self) -> Result<Identity> {
        self.0
            .run_mobile_plugin("getIdentity", ())
            .map_err(|error| Error::Native(error.to_string()))
    }

    /// Signs bytes inside the native Keychain-backed plugin. The private key and the signing
    /// operation never enter Rust or the WebView.
    pub fn sign(&self, message: &[u8]) -> Result<Vec<u8>> {
        let response: SignResponse = self
            .0
            .run_mobile_plugin(
                "sign",
                SignRequest {
                    message: hex(message),
                },
            )
            .map_err(|error| Error::Native(error.to_string()))?;
        decode_hex(&response.signature).map_err(Error::Native)
    }

    pub fn reset(&self) -> Result<Identity> {
        self.0
            .run_mobile_plugin("resetIdentity", ())
            .map_err(|error| Error::Native(error.to_string()))
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_hex(value: &str) -> std::result::Result<Vec<u8>, String> {
    if value.len() % 2 != 0 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("native signature was not valid hexadecimal".into());
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char)
                .to_digit(16)
                .ok_or_else(|| "native signature was not valid hexadecimal".to_string())?;
            let low = (pair[1] as char)
                .to_digit(16)
                .ok_or_else(|| "native signature was not valid hexadecimal".to_string())?;
            Ok(((high << 4) | low) as u8)
        })
        .collect()
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("device-identity")
        .setup(|app, api| {
            #[cfg(target_os = "ios")]
            let handle = api.register_ios_plugin(init_plugin_device_identity)?;
            #[cfg(target_os = "android")]
            compile_error!("Sjel device identity currently supports iOS only.");
            app.manage(DeviceIdentity(handle));
            Ok(())
        })
        .build()
}
