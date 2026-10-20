//! The device key, held by `host/DeviceIdentity.swift` in the iOS Keychain. Rust gets the public
//! key and signatures; the private key never crosses into this process's Rust side.

#[cfg(target_os = "ios")]
#[allow(unsafe_code)]
mod ffi {
    unsafe extern "C" {
        /// Writes 32 bytes to `out`. Returns 0, or -1 when Keychain refused.
        pub fn sjel_device_public_key(out: *mut u8) -> i32;
        /// Signs `length` bytes at `message`, writes 64 bytes to `out`. Returns 0, or -1.
        pub fn sjel_device_sign(message: *const u8, length: usize, out: *mut u8) -> i32;
    }

    pub fn public_key() -> Option<[u8; 32]> {
        let mut out = [0u8; 32];
        // SAFETY: `out` is a live 32-byte buffer, the exact size the Swift side writes
        // (`sjelDevicePublicKey` copies `count: 32`). The call does not keep the pointer.
        (unsafe { sjel_device_public_key(out.as_mut_ptr()) } == 0).then_some(out)
    }

    pub fn sign(message: &[u8]) -> Option<[u8; 64]> {
        let mut out = [0u8; 64];
        // SAFETY: `message` is valid for `message.len()` bytes and `out` for the 64 bytes the
        // Swift side writes (`sjelDeviceSign` copies `count: 64`). Neither pointer is kept.
        (unsafe { sjel_device_sign(message.as_ptr(), message.len(), out.as_mut_ptr()) } == 0)
            .then_some(out)
    }
}

const NO_KEY: &str = "The device key in Keychain is not available.";

/// The public key, hex, as the claim sends it.
pub fn public_key() -> Result<String, String> {
    #[cfg(target_os = "ios")]
    return ffi::public_key()
        .map(|key| device_auth::hex(&key))
        .ok_or_else(|| NO_KEY.into());
    #[cfg(not(target_os = "ios"))]
    Err(NO_KEY.into())
}

/// Signs `message` with the device key: a [`sjel_gpui_core::device::Signer`].
pub fn sign(message: &[u8]) -> Result<Vec<u8>, String> {
    #[cfg(target_os = "ios")]
    return ffi::sign(message)
        .map(|signature| signature.to_vec())
        .ok_or_else(|| NO_KEY.into());
    #[cfg(not(target_os = "ios"))]
    {
        let _ = message;
        Err(NO_KEY.into())
    }
}
