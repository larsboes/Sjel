#[cfg(mobile)]
mod device_identity;
mod mac_bridge;
mod mirror;
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
            mirror::mirror_status,
            mirror::mirror_sync,
            mirror::mirror_tasks,
            mirror::mirror_records,
            mirror::mirror_projects,
            mirror::mirror_project_records,
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
        mirror::mirror_status,
        mirror::mirror_sync,
        mirror::mirror_tasks,
        mirror::mirror_records,
        mirror::mirror_projects,
        mirror::mirror_project_records,
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
            spawn_mirror_sync(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Fetches the device's copy of the Mac's data once, shortly after start.
///
/// The manifest is a few hundred bytes and the copy crosses only when what the Mac built differs
/// from what the device holds, so this is cheap on a start that has nothing to do. It runs here
/// because no page reads the copy yet: without this, `mirror_sync` would be unreachable code and the
/// copy would never exist on a device. A page that wants it fresh calls the same command.
///
/// A failure is logged and nothing else: an unreachable Mac leaves whatever copy is already stored,
/// and its age is what `mirror_status` reports.
fn spawn_mirror_sync<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    std::thread::spawn(move || {
        // Let the connection settings and this device's identity settle before the first request.
        std::thread::sleep(Duration::from_secs(5));
        tauri::async_runtime::block_on(async {
            let mirror = match mirror::Mirror::open(&app) {
                Ok(mirror) => mirror,
                Err(error) => {
                    log::warn!("mirror: {error}");
                    return;
                }
            };
            let transport = match mac_bridge::transport(&app) {
                Ok(transport) => transport,
                // Not configured yet: the app was never paired with a node, which is not a failure.
                Err(_) => return,
            };
            match mirror.sync(&transport, true).await {
                Ok(outcome) => log::info!(
                    "mirror: {} ({} bytes fetched, {} rows in the copy)",
                    outcome.action,
                    outcome.transferred,
                    outcome.status.copy.as_ref().map_or(0, |copy| copy.rows)
                ),
                Err(error) => log::warn!("mirror: {error}"),
            }
        });
    });
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
