//! Regenerate the registry's TypeScript contracts or check their freshness.

use std::{error::Error, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    let check = match std::env::args().nth(1).as_deref() {
        None => false,
        Some("--check") => true,
        _ => return Err("usage: typescript [--check]".into()),
    };
    let generated = projects_api::registry::typescript()?;
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/projects.generated.ts");
    if check {
        if std::fs::read_to_string(&path)? != generated {
            return Err("Projects TypeScript is stale: run sjel cargo run -p projects-api --example typescript".into());
        }
    } else {
        std::fs::write(path, generated)?;
    }
    Ok(())
}
