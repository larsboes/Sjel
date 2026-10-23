//! Render the experimental basemap without opening a window or reading personal trip data.
#[cfg(target_os = "macos")]
#[path = "../src/travel/native_renderer.rs"]
mod native_renderer;

#[cfg(target_os = "macos")]
fn main() -> Result<(), String> {
    let destination = std::env::args_os()
        .nth(1)
        .ok_or("Supply an output PNG path.")?;
    std::thread::spawn(move || {
        let mut renderer = native_renderer::Renderer::new(2)?;
        let image = renderer.render(native_renderer::Camera {
            latitude: 52.52,
            longitude: 13.405,
            zoom: 13.,
            scale: 2,
            width: 1000,
            height: 600,
        })?;
        image
            .save(destination)
            .map_err(|error| format!("Saving map: {error}"))?;
        println!(
            "Rendered Berlin: {} × {} pixels",
            image.width(),
            image.height()
        );
        Ok(())
    })
    .join()
    .map_err(|_| "Native map worker panicked.".to_string())?
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("The native-map prototype currently supports macOS only.");
    std::process::exit(1);
}
