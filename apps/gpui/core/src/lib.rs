//! What the Mac app (`apps/gpui`) and the iOS app (`ios/`) share: the destinations
//! of the web dashboard, the client for the Sjel shell, and a paired device's client. No GPUI
//! types, so either platform can build it.

pub mod device;
pub mod route;
pub mod shell;
