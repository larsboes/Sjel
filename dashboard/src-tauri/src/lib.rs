#[cfg(mobile)]
mod device_identity;
mod mac_bridge;
mod sync;

use std::time::Duration;

use tauri::Manager;

// The Swift archives are listed before Rust dependency archives by rustc.
// Keep one direct reference in the app object so ld extracts each custom
// plugin's own Swift object without force-loading its bundled Tauri runtime
// objects (which would create duplicate symbols).
#[cfg(target_os = "ios")]
mod ios_link_anchors {
    unsafe extern "C" {
        fn init_plugin_device_identity();
        fn init_plugin_roomplan();
    }

    #[used]
    static DEVICE_IDENTITY: unsafe extern "C" fn() = init_plugin_device_identity;
    #[used]
    static ROOMPLAN: unsafe extern "C" fn() = init_plugin_roomplan;
}

/// How often pending edits are retried while any wait (sync step 2, item 6).
const FLUSH_INTERVAL: Duration = Duration::from_secs(30);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(mobile)]
    let builder = builder
        .plugin(tauri_plugin_roomplan::init())
        .plugin(tauri_plugin_device_identity::init())
        .plugin(tauri_plugin_foundation_models::init())
        .plugin(tauri_plugin_local_network::init())
        .invoke_handler(tauri::generate_handler![
            mac_bridge::mac_request,
            mac_bridge::mac_request_bytes,
            mac_bridge::connection_settings_get,
            mac_bridge::connection_settings_set,
            mac_bridge::connection_local_set,
            mac_bridge::sync_status,
            mac_bridge::sync_entries,
            mac_bridge::sync_flush,
            mac_bridge::sync_resolve,
            device_identity::device_identity_get,
            device_identity::device_identity_reset,
        ]);

    #[cfg(not(mobile))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        mac_bridge::mac_request,
        mac_bridge::mac_request_bytes,
        mac_bridge::connection_settings_get,
        mac_bridge::connection_settings_set,
        mac_bridge::connection_local_set,
        mac_bridge::sync_status,
        mac_bridge::sync_entries,
        mac_bridge::sync_flush,
        mac_bridge::sync_resolve,
    ]);

    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_barcode_scanner::init());

    builder
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            app.manage(mac_bridge::Local::open(app.handle()));
            spawn_flush_loop(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Flushes pending edits once at start and then every [`FLUSH_INTERVAL`] while any wait. The
/// foreground flush and "Sync now" come from the page (`sync_flush`). On iOS the thread is
/// suspended with the app, so this does not run in the background.
fn spawn_flush_loop<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    std::thread::spawn(move || loop {
        let local = app.state::<mac_bridge::Local>();
        if let Some(store) = &local.store {
            if store.has_pending() {
                if let Ok(t) = mac_bridge::transport(&app) {
                    let now = mac_bridge::now_ms();
                    #[cfg(mobile)]
                    {
                        if let (Ok(settings), Ok(identity)) = (
                            mac_bridge::connection_settings(&app),
                            app.state::<tauri_plugin_device_identity::DeviceIdentity<R>>()
                                .get(),
                        ) {
                            tauri::async_runtime::block_on(sync::flush_protocol(
                                store,
                                &t,
                                now,
                                &format!("node_{}", identity.id),
                                &settings.node_id,
                                &identity.id,
                            ));
                        }
                    }
                    #[cfg(not(mobile))]
                    {
                        tauri::async_runtime::block_on(sync::flush(store, &t, now));
                    }
                }
            }
        }
        std::thread::sleep(FLUSH_INTERVAL);
    });
}
