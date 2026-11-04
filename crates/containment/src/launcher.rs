use crate::seatbelt::SeatbeltProfile;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Environment sanitation for contained executions.
pub fn sanitize_environment(
    inherit: impl IntoIterator<Item = (String, String)>,
    allowed_provider_env: Option<(&str, &str)>,
) -> HashMap<String, String> {
    let mut clean = HashMap::new();

    for (k, v) in inherit {
        let upper = k.to_uppercase();

        // Strip SSH socket
        if upper == "SSH_AUTH_SOCK" {
            continue;
        }

        // Strip tokens, secrets, keys, and passwords
        if upper.ends_with("_TOKEN")
            || upper.ends_with("_KEY")
            || upper.ends_with("_SECRET")
            || upper.ends_with("_PASSWORD")
            || upper.ends_with("_CREDENTIALS")
            || upper.starts_with("AWS_")
            || upper.starts_with("GCP_")
            || upper.starts_with("GOOGLE_")
            || upper.starts_with("AZURE_")
        {
            continue;
        }

        clean.insert(k, v);
    }

    if let Some((k, v)) = allowed_provider_env {
        clean.insert(k.to_string(), v.to_string());
    }

    clean
}

/// Command builder for running under macOS sandbox-exec.
pub struct ContainedCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub profile: SeatbeltProfile,
    pub proxy_port: Option<u16>,
}

impl ContainedCommand {
    pub fn new(program: impl AsRef<Path>, profile: SeatbeltProfile) -> Self {
        Self {
            program: program.as_ref().to_path_buf(),
            args: Vec::new(),
            env: sanitize_environment(std::env::vars(), None),
            profile,
            proxy_port: None,
        }
    }

    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for a in args {
            self.args.push(a.into());
        }
        self
    }

    pub fn with_proxy_port(mut self, port: u16) -> Self {
        self.proxy_port = Some(port);
        let proxy_url = format!("http://127.0.0.1:{port}");
        self.env.insert("HTTP_PROXY".into(), proxy_url.clone());
        self.env.insert("HTTPS_PROXY".into(), proxy_url.clone());
        self.env.insert("ALL_PROXY".into(), proxy_url.clone());
        self.env.insert("http_proxy".into(), proxy_url.clone());
        self.env.insert("https_proxy".into(), proxy_url);
        self
    }

    pub fn build_command(&self) -> Command {
        #[cfg(target_os = "macos")]
        {
            let mut cmd = Command::new("/usr/bin/sandbox-exec");
            cmd.arg("-p");
            cmd.arg(self.profile.render());
            cmd.arg(&self.program);
            cmd.args(&self.args);
            cmd.env_clear();
            cmd.envs(&self.env);
            cmd
        }
        #[cfg(not(target_os = "macos"))]
        {
            let mut cmd = Command::new(&self.program);
            cmd.args(&self.args);
            cmd.env_clear();
            cmd.envs(&self.env);
            cmd
        }
    }
}
