//! Interior reads and writes its files under `SJEL_DATA_ROOT_INTERIOR` when that root is set, and
//! refuses to work when the root is unavailable instead of falling back to the overlay.
//!
//! One test function, because the settings are process-wide. The catalogue comes from a mock
//! Inventory listener, so the test opens no database.

use interior::layout_io;
use interior::model::{self, Model, ModelError};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn fixture_flat() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overlay/data/interior/flats/muster")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Every file under `root` with its bytes, so a test can prove a tree did not change.
fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// A loopback listener that answers every request with an empty catalogue and counts requests.
fn mock_inventory() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0u8; 8192];
            let _ = stream.read(&mut buffer);
            counter.fetch_add(1, Ordering::SeqCst);
            let body = "[]";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (url, hits)
}

#[test]
fn the_external_root_owns_reads_and_writes_and_an_unavailable_root_fails() {
    let base = std::env::temp_dir().join(format!("interior-data-root-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let overlay = base.join("overlay");
    let external = base.join("external");
    copy_dir(
        &fixture_flat(),
        &overlay.join("data/interior/flats/overlay-only"),
    );
    copy_dir(
        &fixture_flat(),
        &external.join("data/interior/flats/muster"),
    );
    let overlay_before = snapshot(&overlay);

    let (url, hits) = mock_inventory();
    std::env::set_var("SJEL_PERSONAL_ROOT", &overlay);
    std::env::set_var("SJEL_DATA_ROOT_INTERIOR", &external);
    std::env::set_var("SJEL_INVENTORY_URL", &url);
    std::env::remove_var("SJEL_INTERIOR_FLAT");

    assert_eq!(model::data_dir().unwrap(), external.join("data/interior"));
    assert_eq!(model::flats().unwrap(), vec!["muster".to_string()]);
    assert_eq!(model::default_flat().unwrap(), "muster");

    let loaded = Model::load("muster").expect("the flat loads from the external root");
    assert!(
        hits.load(Ordering::SeqCst) >= 1,
        "the catalogue came from Inventory"
    );
    assert_eq!(loaded.flat_dir, external.join("data/interior/flats/muster"));

    let layout = loaded
        .layout_names()
        .unwrap()
        .into_iter()
        .next()
        .expect("a layout");
    layout_io::archive_layout(&loaded, &layout).expect("the layout archives");
    let archived = external
        .join("data/interior/flats/muster/layouts")
        .join(layout_io::ARCHIVE)
        .join(format!("{layout}.toml"));
    assert!(archived.is_file(), "the write landed in the external root");
    assert_eq!(
        snapshot(&overlay),
        overlay_before,
        "the overlay was not touched"
    );

    let unmounted = base.join("unmounted");
    std::env::set_var("SJEL_DATA_ROOT_INTERIOR", &unmounted);
    let refusal = model::flats().expect_err("an unavailable root is an error");
    assert!(matches!(refusal, ModelError::DataRoot(_)));
    let message = refusal.to_string();
    assert!(message.contains("SJEL_DATA_ROOT_INTERIOR"), "{message}");
    assert!(message.contains(unmounted.to_str().unwrap()), "{message}");
    assert!(
        Model::load("overlay-only").is_err(),
        "no fallback to the overlay"
    );

    std::env::remove_var("SJEL_DATA_ROOT_INTERIOR");
    assert_eq!(model::flats().unwrap(), vec!["overlay-only".to_string()]);

    let _ = std::fs::remove_dir_all(&base);
}
