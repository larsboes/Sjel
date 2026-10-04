//! `sjel-agent [--role <name>] [--continue | --session <file>] [<prompt...>]`
//!
//! A coding agent in the working directory. With a prompt it runs once and exits. Without one
//! it reads prompts from stdin, one per line, and keeps the conversation between them.
//! `/exit` or end of input (Ctrl-D) stops it.
//!
//! Ctrl-C stops the current turn and keeps the conversation. A second Ctrl-C before the next
//! turn starts exits with 130, which is also the way out of a model that streams nothing.
//!
//! Every turn is appended to a session file in the private overlay,
//! `<overlay>/data/agent/sessions/<working directory>/<millis>.jsonl`. `--continue` resumes the
//! newest session for this directory. `--session <file>` resumes or starts that file.
//!
//! The model comes from the overlay's `config/inference.json`, role `coding` unless `--role`
//! names another. `AGENTS.md` in the working directory, when present, is appended to the system
//! prompt. The answer streams to stdout. Thinking, tool calls and their results go to stderr,
//! so `sjel-agent "..." > answer.md` keeps only the answer.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use sjel_agent::{session, tools, Agent, AgentError, Event, Message};

const DEFAULT_ROLE: &str = "coding";
/// How much of a tool result the trace on stderr shows. The model still gets all of it.
const TRACE_LIMIT: usize = 400;
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
/// The status a shell reports for a process ended by SIGINT.
const INTERRUPTED: u8 = 130;

struct Args {
    role: String,
    resume: Resume,
    prompt: String,
}

enum Resume {
    New,
    Latest,
    File(PathBuf),
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut parsed = Args {
        role: DEFAULT_ROLE.to_owned(),
        resume: Resume::New,
        prompt: String::new(),
    };
    let mut prompt = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--role" => parsed.role = args.next().ok_or("--role needs a role name")?,
            "--continue" | "-c" => parsed.resume = Resume::Latest,
            "--session" => {
                parsed.resume = Resume::File(args.next().ok_or("--session needs a file")?.into())
            }
            _ => prompt.push(arg),
        }
    }
    parsed.prompt = prompt.join(" ");
    Ok(parsed)
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("sjel-agent: {e}");
            eprintln!(
                "usage: sjel-agent [--role <name>] [--continue | --session <file>] [<prompt...>]"
            );
            return ExitCode::from(2);
        }
    };
    match run(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("sjel-agent: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> Result<ExitCode, String> {
    let role = sjel_inference::InferenceConfig::load(sjel_config::overlay_config)
        .role(&args.role)
        .ok_or_else(|| {
            format!(
                "the inference role `{0}` is not configured. Add it to the overlay's \
                 config/inference.json, for example \"{0}\": {{ \"backend\": \"omlx\", \"model\": \"...\" }}",
                args.role
            )
        })?;
    let cwd = std::env::current_dir().map_err(|e| format!("no working directory: {e}"))?;

    // The shutdown goes first: it exits only when the flag is already set, so the first
    // Ctrl-C sets the flag and stops the turn, and the second one exits.
    let stop = Arc::new(AtomicBool::new(false));
    let sigint = signal_hook::consts::SIGINT;
    signal_hook::flag::register_conditional_shutdown(
        sigint,
        i32::from(INTERRUPTED),
        Arc::clone(&stop),
    )
    .and_then(|_| signal_hook::flag::register(sigint, Arc::clone(&stop)))
    .map_err(|e| format!("could not handle Ctrl-C: {e}"))?;

    let agent = Agent::new(role, tools::coding(&cwd, &stop), Arc::clone(&stop))
        .map_err(|e| e.to_string())?;

    let session_dir = sjel_config::overlay_data_dir("agent").map(|d| {
        d.join("sessions")
            .join(cwd.to_string_lossy().replace('/', "-"))
    });
    let session_path = match (&args.resume, &session_dir) {
        (Resume::File(path), _) => Some(path.clone()),
        (Resume::Latest, Some(dir)) => Some(
            session::latest(dir)
                .ok_or_else(|| format!("no session to continue in {}", dir.display()))?,
        ),
        (Resume::Latest, None) => {
            return Err("--continue needs an overlay (SJEL_PERSONAL_ROOT)".into())
        }
        (Resume::New, dir) => dir.as_deref().map(session::new_path),
    };

    let mut messages = vec![Message::System {
        content: system_prompt(&cwd),
    }];
    if let Some(path) = session_path.as_deref().filter(|p| p.exists()) {
        messages.extend(session::load(path)?);
    }
    // Everything up to here is on disk already, or is the system prompt, which is never stored.
    let mut saved = messages.len();
    match &session_path {
        Some(path) => eprintln!("{DIM}[{}] {}{RESET}", agent.model(), path.display()),
        None => eprintln!(
            "{DIM}[{}] no overlay, so this session is not saved{RESET}",
            agent.model()
        ),
    }
    let mut save = |messages: &[Message]| {
        if let Some(path) = &session_path {
            if let Err(e) = session::append(path, &messages[saved..]) {
                eprintln!(
                    "sjel-agent: could not save the session to {}: {e}",
                    path.display()
                );
            }
        }
        saved = messages.len();
    };

    if !args.prompt.trim().is_empty() {
        let result = turn(&agent, &mut messages, args.prompt);
        save(&messages);
        return match result {
            Ok(()) => Ok(ExitCode::SUCCESS),
            Err(AgentError::Stopped) => Ok(ExitCode::from(INTERRUPTED)),
            Err(e) => Err(e.to_string()),
        };
    }

    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        eprint!("\n> ");
        line.clear();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => return Ok(ExitCode::SUCCESS),
            Ok(_) => {}
            // A Ctrl-C at the prompt interrupts the read. The flag is set now, so a second
            // Ctrl-C exits; anything else typed clears it again below.
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.to_string()),
        }
        match line.trim() {
            "" => continue,
            "/exit" => return Ok(ExitCode::SUCCESS),
            prompt => {
                stop.store(false, Ordering::Relaxed);
                let before = messages.len();
                match turn(&agent, &mut messages, prompt.to_owned()) {
                    Ok(()) => {}
                    Err(AgentError::Stopped) => eprintln!("{DIM}[stopped]{RESET}"),
                    // Any other failure leaves the conversation as it was, so the prompt can
                    // be sent again.
                    Err(e) => {
                        eprintln!("sjel-agent: {e}");
                        messages.truncate(before);
                    }
                }
                save(&messages);
            }
        }
    }
}

fn system_prompt(cwd: &Path) -> String {
    let mut system = format!(
        "You are a coding agent working in {}. Use the tools to read, search and change files \
         and to run commands. Use grep and find to search, not bash. Request independent reads \
         and searches together in one turn: they run in parallel. Read a file before you edit \
         it. When the task is done, answer with a short summary of what you changed.",
        cwd.display()
    );
    if let Ok(agents_md) = std::fs::read_to_string(cwd.join("AGENTS.md")) {
        system.push_str("\n\n# AGENTS.md\n\n");
        system.push_str(&agents_md);
    }
    system
}

fn turn(agent: &Agent, messages: &mut Vec<Message>, prompt: String) -> Result<(), AgentError> {
    messages.push(Message::User { content: prompt });
    let mut out = std::io::stdout().lock();
    let result = agent.run(messages, |event| {
        // Write errors on the trace are not worth stopping the run for.
        match event {
            Event::Text(text) => {
                let _ = out.write_all(text.as_bytes());
                let _ = out.flush();
            }
            Event::Reasoning(text) => eprint!("{DIM}{text}{RESET}"),
            Event::ToolCall(call) => eprintln!(
                "\n{DIM}> {}({}){RESET}",
                call.function.name, call.function.arguments
            ),
            Event::ToolResult { content, .. } => {
                eprintln!("{DIM}{}{RESET}", tools::cap(content, TRACE_LIMIT))
            }
        }
    });
    let _ = writeln!(out);
    result
}
