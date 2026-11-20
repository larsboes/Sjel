use std::path::Path;

use projects::{commands, jobs::CommandResult};

type Fallible<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("edit") => edit(&args),
        Some(_) => absolute_paths(&args)
            .and_then(|args| execute(&args))
            .and_then(|r| print(&r)),
        None => Err(commands::USAGE.into()),
    };
    if let Err(error) = result {
        eprintln!("projects: {error}");
        std::process::exit(1);
    }
}

/// Run a command in the owner when one answers, otherwise against a local store.
fn execute(args: &[String]) -> Fallible<CommandResult> {
    if let Some(result) = projects::jobs::request(args)? {
        return Ok(result);
    }
    local(args)
}

#[cfg(feature = "turso-wal")]
fn local(_args: &[String]) -> Fallible<CommandResult> {
    Err(
        "the Projects owner is not running; start it with `tools/service-runner.sh start projects`"
            .into(),
    )
}

#[cfg(not(feature = "turso-wal"))]
fn local(args: &[String]) -> Fallible<CommandResult> {
    // After the cutover the shared tables are stale, so only the owner may answer.
    if projects::owner_file().is_some() {
        return Err("the Projects owner is not running; start it with `tools/service-runner.sh start projects`".into());
    }
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let projects = projects::open_default().await?;
        let mut result = CommandResult::default();
        result.error = commands::run(&projects, args, &mut result.stdout, &mut result.stderr)
            .await
            .err()
            .map(|error| error.to_string());
        Ok(result)
    })
}

/// Print the command's output and turn its refusal into this process's error.
fn print(result: &CommandResult) -> Fallible<()> {
    eprint!("{}", result.stderr);
    print!("{}", result.stdout);
    match &result.error {
        Some(error) => Err(error.clone().into()),
        None => Ok(()),
    }
}

/// The owner runs in another working directory, so file arguments travel as absolute paths.
fn absolute_paths(args: &[String]) -> Fallible<Vec<String>> {
    let sub = args.get(1).map(String::as_str);
    let lesson = args.get(2).map(String::as_str);
    let positions: &[usize] = match (args[0].as_str(), sub, lesson) {
        ("put" | "attach", ..) => &[2],
        ("import" | "freeze-vault", ..) => &[1],
        ("workspace", Some("adopt"), _) => &[2],
        ("workspace", Some("setup"), _) => &[3],
        ("workspace", Some("lesson"), Some("publish")) => &[4, 5],
        _ => &[],
    };
    let flags: &[&str] = match (args[0].as_str(), sub) {
        ("put", _) => &["--base"],
        ("workspace", Some("lesson")) => &["--proposal"],
        _ => &[],
    };
    let mut out = args.to_vec();
    for &i in positions {
        if let Some(arg) = out.get_mut(i) {
            *arg = absolute(arg)?;
        }
    }
    for flag in flags {
        if let Some(i) = out.iter().position(|a| a == flag) {
            if let Some(arg) = out.get_mut(i + 1) {
                *arg = absolute(arg)?;
            }
        }
    }
    Ok(out)
}

fn absolute(path: &str) -> Fallible<String> {
    Ok(std::path::absolute(Path::new(path))?
        .to_string_lossy()
        .into_owned())
}

/// Edit a record in `$EDITOR`, then save it only while the stored content still matches.
fn edit(args: &[String]) -> Fallible<()> {
    let path = args.get(1).ok_or(commands::USAGE)?;
    let shown = execute(&["show".into(), path.clone()])?;
    if let Some(error) = shown.error {
        return Err(error.into());
    }
    let base = shown.stdout;
    let scratch = std::env::temp_dir().join(format!("sjel-projects-edit-{}", std::process::id()));
    std::fs::create_dir_all(&scratch)?;
    let (base_file, edited_file) = (scratch.join("base.md"), scratch.join("edited.md"));
    std::fs::write(&base_file, &base)?;
    std::fs::write(&edited_file, &base)?;
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".into());
    let status = std::process::Command::new(&editor)
        .arg(&edited_file)
        .status();
    let saved = (|| {
        let status = status?;
        if !status.success() {
            return Err(format!("{editor} exited with {status}; nothing saved").into());
        }
        if std::fs::read_to_string(&edited_file)? == base {
            println!("unchanged {path}");
            return Ok(());
        }
        let put = [
            "put".to_owned(),
            path.clone(),
            edited_file.to_string_lossy().into_owned(),
            "--base".to_owned(),
            base_file.to_string_lossy().into_owned(),
        ];
        print(&execute(&put)?)
    })();
    let _ = std::fs::remove_dir_all(&scratch);
    saved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| (*a).to_owned()).collect()
    }

    #[test]
    fn file_arguments_become_absolute_and_record_paths_stay_relative() {
        let here = std::env::current_dir().unwrap();
        let abs = |p: &str| here.join(p).to_string_lossy().into_owned();
        let put = absolute_paths(&strings(&[
            "put",
            "Pilot/Note.md",
            "new.md",
            "--base",
            "old.md",
        ]))
        .unwrap();
        assert_eq!(
            put,
            [
                "put".into(),
                "Pilot/Note.md".into(),
                abs("new.md"),
                "--base".into(),
                abs("old.md")
            ]
        );
        let set = absolute_paths(&strings(&["set", "Pilot/Note.md", "status", "done"])).unwrap();
        assert_eq!(set, strings(&["set", "Pilot/Note.md", "status", "done"]));
        let import =
            absolute_paths(&strings(&["import", "vault/Projects", "--only", "Pilot"])).unwrap();
        assert_eq!(import[1], abs("vault/Projects"));
        assert_eq!(import[3], "Pilot");
        let publish = absolute_paths(&strings(&[
            "workspace",
            "lesson",
            "publish",
            "Pilot/L.md",
            "review.md",
            "vault",
            "K/L.md",
            "--declassify",
            "why",
        ]))
        .unwrap();
        assert_eq!(
            (publish[4].as_str(), publish[5].as_str()),
            (abs("review.md").as_str(), abs("vault").as_str())
        );
        assert_eq!(publish[6], "K/L.md");
    }
}
