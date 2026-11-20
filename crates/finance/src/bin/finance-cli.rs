fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut output = String::new();
    let outcome = run(&args, &mut output);
    print!("{output}");
    match outcome {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(error) if error == finance::commands::USAGE => {
            eprintln!("{error}");
            std::process::exit(2);
        }
        Err(error) => {
            eprintln!("finance: {error}");
            std::process::exit(1);
        }
    }
}

fn run(args: &[String], output: &mut String) -> Result<bool, String> {
    #[cfg(feature = "turso-wal")]
    if let Some(result) = finance::jobs::request(args, output)? {
        return Ok(result);
    }
    finance::commands::run(args, output)
}
