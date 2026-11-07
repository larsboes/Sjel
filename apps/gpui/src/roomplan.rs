//! The Mac's RoomPlan preview: the scan is cached under Application Support and opened in Quick
//! Look. The shared Inventory page receives it as a handler (`InventoryPage::set_cache_preview_handler`).

pub fn cache_preview(revision_id: &str, draft_json: &str, asset_bytes: Vec<u8>) {
    let revision_id = revision_id.to_owned();
    let draft_json = draft_json.to_owned();
    std::thread::spawn(move || {
        if revision_id.is_empty()
            || !revision_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return;
        }
        let Ok(home) = std::env::var("HOME") else {
            return;
        };
        let root =
            std::path::PathBuf::from(home).join("Library/Application Support/dev.sjel/roomplan");
        let directory = root.join(&revision_id);
        let asset = directory.join("parametric.usdz");
        if directory.exists() {
            let existing_draft = std::fs::read(directory.join("draft.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
            if existing_draft
                .as_ref()
                .and_then(|draft| draft.get("draft_id"))
                .and_then(serde_json::Value::as_str)
                != Some(revision_id.as_str())
                || std::fs::read(&asset).ok().as_deref() != Some(asset_bytes.as_slice())
            {
                return;
            }
            let draft_path = directory.join("draft.json");
            let draft_temp = directory.join(".draft.json.tmp");
            if std::fs::write(&draft_temp, draft_json).is_err()
                || std::fs::rename(&draft_temp, &draft_path).is_err()
            {
                let _ = std::fs::remove_file(draft_temp);
                return;
            }
        } else {
            if serde_json::from_str::<serde_json::Value>(&draft_json)
                .ok()
                .and_then(|draft| {
                    draft
                        .get("draft_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .as_deref()
                != Some(revision_id.as_str())
            {
                return;
            }
            let staging = root.join(format!(".{}.{}.tmp", revision_id, std::process::id()));
            if std::fs::create_dir_all(&root).is_err()
                || std::fs::create_dir(&staging).is_err()
                || std::fs::write(staging.join("draft.json"), draft_json).is_err()
                || std::fs::write(staging.join("parametric.usdz"), asset_bytes).is_err()
            {
                let _ = std::fs::remove_dir_all(&staging);
                return;
            }
            if std::fs::rename(&staging, &directory).is_err() {
                let _ = std::fs::remove_dir_all(&staging);
                return;
            }
        }
        let _ = std::process::Command::new("/usr/bin/qlmanage")
            .args(["-p"])
            .arg(asset)
            .spawn();
    });
}
