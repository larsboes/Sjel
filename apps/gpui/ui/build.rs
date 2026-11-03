use std::fs;
use std::path::{Path, PathBuf};

fn collect(dir: &Path, root: &Path, out: &mut Vec<(String, String, String, String)>) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, root, out);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            println!("cargo:rerun-if-changed={}", path.display());
            let relative = path.strip_prefix(root).unwrap_or(&path);
            let mut slug = relative
                .with_extension("")
                .to_string_lossy()
                .replace('\\', "/");
            if slug == "README" {
                slug = "index".into();
            } else if let Some(parent) = slug.strip_suffix("/index") {
                slug = parent.to_owned();
            }
            let category = slug.split('/').next().unwrap_or("general").to_owned();
            let file = format!("docs/{}", relative.to_string_lossy().replace('\\', "/"));
            out.push((slug, category, file, path.to_string_lossy().into_owned()));
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let root = manifest
        .join("../../../docs")
        .canonicalize()
        .expect("docs directory");
    let mut docs = Vec::new();
    collect(&root, &root, &mut docs);
    docs.sort_by(|a, b| a.0.cmp(&b.0));
    let rows = docs
        .iter()
        .map(|(slug, category, file, path)| {
            format!(
                "DocSource {{ slug: {slug:?}, category: {category:?}, file: {file:?}, markdown: include_str!({path:?}) }}",
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    let generated = format!("static DOCS: &[DocSource] = &[{rows}];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("out dir"));
    fs::write(out.join("docs.rs"), generated).expect("write generated docs index");
}
