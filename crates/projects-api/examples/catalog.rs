//! Export the same declaration served by Projects for frontend generation.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        serde_json::to_string_pretty(&projects_api::registry::declaration())?
    );
    Ok(())
}
