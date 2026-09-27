use super::*;

/// Reap frequently enough that overshoot stays below the smallest sane timeout.
pub(crate) const REAP_INTERVAL: Duration = Duration::from_secs(30);

/// Owns the idle-panel loop for exactly as long as the status server lives.
pub(crate) struct IdlePanelReaper(tokio::task::JoinHandle<()>);

impl IdlePanelReaper {
    pub(crate) fn start() -> Option<Self> {
        let root = match axon_root() {
            Ok(root) => root,
            Err(error) => {
                eprintln!("idle reaper not started: {error}");
                return None;
            }
        };
        Some(Self(tokio::spawn(async move {
            let client = reqwest::Client::new();
            loop {
                tokio::time::sleep(REAP_INTERVAL).await;
                reap_idle_panels(&client, &root).await;
            }
        })))
    }
}

impl Drop for IdlePanelReaper {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Seconds since a visible tab last said it was there, asked of the panel itself.
///
/// `None` on any failure, and that is the whole safety property: the panel not
/// answering means this process cannot tell whether somebody is reading it, and the
/// only acceptable answer to "I don't know" is to leave it alone. A panel served by
/// something other than tools/panel-server.ts simply never reports idle and is
/// therefore never reaped, which is the correct behaviour rather than a gap.
pub(crate) async fn panel_idle_seconds(client: &reqwest::Client, svc: &Service) -> Option<u64> {
    let url = format!("http://127.0.0.1:{}/__axon/idle", svc.panel_port);
    let res = client
        .get(&url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    res.json::<Value>()
        .await
        .ok()?
        .get("idle_seconds")?
        .as_u64()
}

/// Stop panels nobody is looking at.
///
/// `idle-stop`, not `stop`: `stop` sets a maintenance hold so a tool can work on a
/// capability's data undisturbed, and an unread page is not a maintenance window. A
/// hold here would make the panel un-startable by anything except the dashboard's
/// resume button until it expired.
pub(crate) async fn reap_idle_panels(client: &reqwest::Client, root: &std::path::Path) {
    let Ok(services) = registry().await else {
        return;
    };
    for svc in services {
        let Some(timeout) = svc.idle_timeout_secs() else {
            continue;
        };
        if !is_up(client, &svc).await {
            continue;
        }
        let Some(idle) = panel_idle_seconds(client, &svc).await else {
            continue;
        };
        if idle < timeout {
            continue;
        }
        let out = tokio::process::Command::new(root.join("tools/service-runner.sh"))
            .arg("idle-stop")
            .arg(&svc.name)
            .output()
            .await;
        match out {
            Ok(o) if o.status.success() => {
                println!(
                    "reaped {} after {idle}s idle (timeout {timeout}s)",
                    svc.name
                )
            }
            Ok(o) => eprintln!(
                "idle-stop {} failed: {}",
                svc.name,
                String::from_utf8_lossy(&o.stderr).trim()
            ),
            Err(e) => eprintln!("could not run service-runner.sh for {}: {e}", svc.name),
        }
    }
}
