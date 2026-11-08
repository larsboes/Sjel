use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "sjel-intake-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn large_binary_assets_are_preserved_and_bound_to_the_digest() {
    let fixture = Fixture::new("asset");
    fs::write(fixture.0.join("SKILL.md"), "Use the sample PDF.").unwrap();
    let bytes = vec![0xff; 2 * 1024 * 1024 + 1];
    fs::write(fixture.0.join("sample.pdf"), &bytes).unwrap();
    let tree = read_tree(&fixture.0).unwrap();
    assert_eq!(tree["sample.pdf"].bytes, bytes);
    let before = digest(&tree);
    let mut changed = bytes;
    changed[0] = 0xfe;
    fs::write(fixture.0.join("sample.pdf"), changed).unwrap();
    assert_ne!(before, digest(&read_tree(&fixture.0).unwrap()));
}

#[test]
fn renaming_oversized_text_as_a_pdf_does_not_bypass_the_text_limit() {
    let fixture = Fixture::new("text");
    fs::write(
        fixture.0.join("sample.pdf"),
        vec![b'a'; 2 * 1024 * 1024 + 1],
    )
    .unwrap();
    assert!(read_tree(&fixture.0).is_err());
}

#[test]
fn symlinked_assets_are_refused() {
    let fixture = Fixture::new("symlink");
    fs::write(fixture.0.join("original"), [0xff]).unwrap();
    std::os::unix::fs::symlink("original", fixture.0.join("sample.pdf")).unwrap();
    assert!(read_tree(&fixture.0).err().unwrap().contains("symlinks"));
}

#[test]
fn package_budget_covers_all_files_together() {
    let fixture = Fixture::new("budget");
    fs::write(fixture.0.join("a"), [0xff; 8]).unwrap();
    fs::write(fixture.0.join("b"), [0xff; 8]).unwrap();
    let mut tree = Tree::new();
    let mut remaining = 15;
    assert!(visit(&fixture.0, &fixture.0, &mut tree, &mut remaining)
        .err()
        .unwrap()
        .contains("package limit"));
}

#[test]
fn binary_file_limit_is_enforced_before_reading() {
    let fixture = Fixture::new("file-limit");
    let file = fs::File::create(fixture.0.join("sample.pdf")).unwrap();
    file.set_len(MAX_FILE_BYTES + 1).unwrap();
    assert!(read_tree(&fixture.0)
        .err()
        .unwrap()
        .contains("screening limit"));
}
