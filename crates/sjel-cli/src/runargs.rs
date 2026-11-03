//! Declared run arguments against a running container, in one canonical form.
//!
//! Ported from tools/lib/runargs.sh on 2026-10-02; the runner was its only caller. Docker and
//! Podman each report bindings in their own shape, so both sides are reduced to sorted
//! `<class> <value>` lines (`port 0.0.0.0:53:53/udp`, `mount src:dst`, `cap NET_ADMIN`,
//! `network default`, `envfile <path>`) and compared per class. A drift report names keys and
//! bindings, never an environment value: the env file holds secrets.

/// `[addr:][hostport:]cport[/proto]` → `addr:hostport:cport/proto`, with `0.0.0.0`, `*` and
/// `tcp` filled in where the spec leaves them out.
pub fn normalize_publish(spec: &str) -> Option<String> {
    if spec.is_empty() {
        return None;
    }
    let (body, proto) = match spec.rsplit_once('/') {
        Some((b, p)) => (b, p),
        None => (spec, "tcp"),
    };
    let parts: Vec<&str> = body.split(':').collect();
    let (addr, hport, cport) = match parts.as_slice() {
        [c] => ("", "", *c),
        [h, c] => ("", *h, *c),
        [a, rest @ ..] => (*a, rest[0], *rest.last().unwrap_or(&"")),
        [] => ("", "", ""),
    };
    let or = |s: &'static str, v: &str| {
        if v.is_empty() {
            s.to_owned()
        } else {
            v.to_owned()
        }
    };
    Some(format!(
        "{}:{}:{cport}/{}",
        or("0.0.0.0", addr),
        or("*", hport),
        or("tcp", proto)
    ))
}

/// `cap_net_raw`, `CAP_NET_RAW` and `NET_RAW` are one capability.
pub fn normalize_cap(c: &str) -> Option<String> {
    if c.is_empty() {
        return None;
    }
    let up = c.to_ascii_uppercase();
    Some(up.strip_prefix("CAP_").unwrap_or(&up).to_owned())
}

/// No network, `default` and `bridge` are the same default network.
pub fn normalize_network(n: &str) -> String {
    match n {
        "" | "default" | "bridge" => "default".to_owned(),
        other => other.to_owned(),
    }
}

/// The canonical lines for a `run` argv, sorted.
pub fn declared_runspec(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    if !args.iter().any(|a| a == "--network") {
        out.push("network default".to_owned());
    }
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        if !matches!(
            flag.as_str(),
            "-p" | "-v" | "--cap-add" | "--network" | "--env-file"
        ) {
            continue;
        }
        let Some(val) = it.next() else { break };
        match flag.as_str() {
            "-p" => out.extend(normalize_publish(val).map(|v| format!("port {v}"))),
            "-v" => out.push(format!("mount {val}")),
            "--cap-add" => out.extend(normalize_cap(val).map(|v| format!("cap {v}"))),
            "--network" => out.push(format!("network {}", normalize_network(val))),
            _ => out.push(format!("envfile {val}")),
        }
    }
    out.sort();
    out
}

/// The canonical lines for `inspect --format '{{json .}}'` output, sorted. Unparseable input
/// yields nothing, as the jq filter it replaced did.
pub fn runspec_from_docker(json: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let s = |v: &serde_json::Value| v.as_str().unwrap_or("").to_owned();
    let mut out = Vec::new();
    let host = &v["HostConfig"];
    if let Some(bindings) = host["PortBindings"].as_object() {
        for (key, list) in bindings {
            let mut cp = key.split('/');
            let port = cp.next().unwrap_or("");
            let proto = cp.next().unwrap_or("tcp");
            for b in list.as_array().into_iter().flatten() {
                let ip = s(&b["HostIp"]);
                let ip = if ip.is_empty() {
                    "0.0.0.0".to_owned()
                } else {
                    ip
                };
                out.push(format!("port {ip}:{}:{port}/{proto}", s(&b["HostPort"])));
            }
        }
    }
    for m in v["Mounts"].as_array().into_iter().flatten() {
        let kind = s(&m["Type"]);
        if kind == "tmpfs" {
            continue;
        }
        let src = if kind == "volume" {
            s(&m["Name"])
        } else {
            s(&m["Source"])
        };
        out.push(format!("mount {src}:{}", s(&m["Destination"])));
    }
    for c in host["CapAdd"].as_array().into_iter().flatten() {
        out.extend(normalize_cap(&s(c)).map(|c| format!("cap {c}")));
    }
    out.push(format!(
        "network {}",
        normalize_network(&s(&host["NetworkMode"]))
    ));
    out.sort();
    out
}

/// The container's environment, one `KEY=VALUE` per entry.
pub fn env_from_docker(json: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| {
            v["Config"]["Env"].as_array().map(|a| {
                a.iter()
                    .filter_map(|e| e.as_str().map(str::to_owned))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// One class compared: `None` when both sides agree, otherwise the report lines.
pub fn runspec_diff(declared: &[String], running: &[String], class: &str) -> Option<String> {
    let pick = |v: &[String]| -> Vec<String> {
        let prefix = format!("{class} ");
        let mut out: Vec<String> = v
            .iter()
            .filter_map(|l| l.strip_prefix(&prefix).map(str::to_owned))
            .collect();
        out.sort();
        out
    };
    let (d, r) = (pick(declared), pick(running));
    // `comm -23` and `comm -13`: a multiset difference over sorted lists.
    let (mut only_d, mut only_r) = (Vec::new(), Vec::new());
    let (mut i, mut j) = (0, 0);
    while i < d.len() || j < r.len() {
        match (d.get(i), r.get(j)) {
            (Some(a), Some(b)) if a == b => {
                i += 1;
                j += 1;
            }
            (Some(a), Some(b)) if a < b => {
                only_d.push(a.clone());
                i += 1;
            }
            (Some(_), Some(b)) => {
                only_r.push(b.clone());
                j += 1;
            }
            (Some(a), None) => {
                only_d.push(a.clone());
                i += 1;
            }
            (None, Some(b)) => {
                only_r.push(b.clone());
                j += 1;
            }
            (None, None) => break,
        }
    }
    if only_d.is_empty() && only_r.is_empty() {
        return None;
    }
    let lines: Vec<String> = only_d
        .iter()
        .map(|l| format!("    declared, not in container: {l}"))
        .chain(
            only_r
                .iter()
                .map(|l| format!("    in container, not declared: {l}")),
        )
        .collect();
    Some(lines.join("\n"))
}

/// Every key the env file declares, against the container's environment. Keys only, never
/// values. Keys the image sets and the file does not are not drift. `(report, ok)`.
pub fn env_diff(envfile: &std::path::Path, running: &[String]) -> (String, bool) {
    let Ok(body) = std::fs::read_to_string(envfile) else {
        return (
            format!("    env file is missing: {}", envfile.display()),
            false,
        );
    };
    let mut lines = Vec::new();
    let mut ok = true;
    for line in body.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, want)) = line.split_once('=') else {
            lines.push(format!(
                "    cannot check (value comes from the host environment): {line}"
            ));
            ok = false;
            continue;
        };
        let prefix = format!("{key}=");
        match running.iter().find_map(|r| r.strip_prefix(&prefix)) {
            Some(have) if have == want => {}
            Some(_) => {
                lines.push(format!("    value differs: {key}"));
                ok = false;
            }
            None => {
                lines.push(format!("    declared, not in container: {key}"));
                ok = false;
            }
        }
    }
    (lines.join("\n"), ok)
}

#[cfg(test)]
mod tests {
    // Moved from tools/runargs.test.sh, one assertion per case it held.
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn publish_specs_normalize() {
        let n = |s| normalize_publish(s).unwrap();
        assert_eq!(n("9090:8080"), "0.0.0.0:9090:8080/tcp");
        assert_eq!(n("127.0.0.1:9090:8080"), "127.0.0.1:9090:8080/tcp");
        assert_eq!(n("53:53/udp"), "0.0.0.0:53:53/udp");
        assert_eq!(n("127.0.0.1:53:53/udp"), "127.0.0.1:53:53/udp");
        assert_eq!(n("8080"), "0.0.0.0:*:8080/tcp");
    }

    #[test]
    fn caps_and_networks_normalize() {
        assert_eq!(normalize_cap("NET_ADMIN").unwrap(), "NET_ADMIN");
        assert_eq!(normalize_cap("CAP_NET_ADMIN").unwrap(), "NET_ADMIN");
        assert_eq!(normalize_cap("cap_net_raw").unwrap(), "NET_RAW");
        assert_eq!(normalize_network(""), "default");
        assert_eq!(normalize_network("bridge"), "default");
        assert_eq!(normalize_network("host"), "host");
    }

    #[test]
    fn a_declared_argv_reduces_to_its_classes() {
        let args = v(&[
            "--name",
            "pihole",
            "--cap-add",
            "NET_ADMIN",
            "-p",
            "53:53/udp",
            "-v",
            "/overlay/data/pihole:/etc/pihole",
            "--env-file",
            "/overlay/env/pihole.env",
        ]);
        assert_eq!(
            declared_runspec(&args),
            v(&[
                "cap NET_ADMIN",
                "envfile /overlay/env/pihole.env",
                "mount /overlay/data/pihole:/etc/pihole",
                "network default",
                "port 0.0.0.0:53:53/udp"
            ])
        );
        let host = v(&[
            "--name",
            "ha",
            "--network",
            "host",
            "--env-file",
            "/overlay/env/ha.env",
        ]);
        assert_eq!(
            declared_runspec(&host),
            v(&["envfile /overlay/env/ha.env", "network host"])
        );
    }

    #[test]
    fn docker_inspect_reduces_to_the_same_classes() {
        let json = r#"{
          "Config": {"Env": ["PATH=/usr/bin", "POSTGRES_PASSWORD=secret-value", "POSTGRES_DB=axon"]},
          "Mounts": [
            {"Type": "volume", "Name": "axon-postgres-data", "Source": "/var/lib/docker/volumes/axon-postgres-data/_data", "Destination": "/var/lib/postgresql/data"},
            {"Type": "bind", "Source": "/overlay/data/pg", "Destination": "/backup"},
            {"Type": "tmpfs", "Source": "", "Destination": "/run"}
          ],
          "HostConfig": {
            "PortBindings": {"5432/tcp": [{"HostIp": "127.0.0.1", "HostPort": "5432"}], "53/udp": [{"HostIp": "", "HostPort": "53"}]},
            "CapAdd": ["NET_ADMIN"],
            "NetworkMode": "bridge"
          }
        }"#;
        assert_eq!(
            runspec_from_docker(json),
            v(&[
                "cap NET_ADMIN",
                "mount /overlay/data/pg:/backup",
                "mount axon-postgres-data:/var/lib/postgresql/data",
                "network default",
                "port 0.0.0.0:53:53/udp",
                "port 127.0.0.1:5432:5432/tcp"
            ])
        );
        assert_eq!(
            runspec_from_docker(r#"{"HostConfig":{}}"#),
            v(&["network default"])
        );
    }

    #[test]
    fn diffs_report_each_side() {
        let same = v(&["port 0.0.0.0:9090:8080/tcp"]);
        assert!(
            runspec_diff(&same, &same, "port").is_none(),
            "identical sets are not drift"
        );
        let d = v(&["port 127.0.0.1:9090:8080/tcp"]);
        let out = runspec_diff(&d, &same, "port").expect("a changed host address is drift");
        assert!(out.contains("declared, not in container: 127.0.0.1:9090:8080/tcp"));
        assert!(out.contains("in container, not declared: 0.0.0.0:9090:8080/tcp"));
        assert!(
            runspec_diff(&v(&["port 0.0.0.0:9091:8080/tcp"]), &same, "port").is_some(),
            "host port"
        );
        assert!(
            runspec_diff(
                &same,
                &v(&["port 0.0.0.0:9090:8080/tcp", "port 0.0.0.0:5000:5000/tcp"]),
                "port"
            )
            .is_some(),
            "removed publish"
        );
        assert!(runspec_diff(
            &v(&["mount axon-pg-data:/var/lib/postgresql/data"]),
            &v(&["mount postgres_data:/var/lib/postgresql"]),
            "mount"
        )
        .is_some());
        assert!(runspec_diff(
            &v(&["cap NET_ADMIN", "cap NET_RAW"]),
            &v(&["cap NET_ADMIN"]),
            "cap"
        )
        .is_some());
        assert!(runspec_diff(&v(&["network host"]), &v(&["network default"]), "network").is_some());
        assert!(
            runspec_diff(
                &v(&["port 0.0.0.0:1:1/tcp", "cap NET_ADMIN"]),
                &v(&["port 0.0.0.0:2:2/tcp", "cap NET_ADMIN"]),
                "cap"
            )
            .is_none(),
            "an unchanged class"
        );
    }

    #[test]
    fn env_drift_names_keys_never_values() {
        let dir = std::env::temp_dir().join(format!("sjel-cli-runargs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let secret = "hunter2-do-not-print";
        let env = dir.join("env");
        std::fs::write(
            &env,
            format!("POSTGRES_DB=axon\nPOSTGRES_PASSWORD={secret}\n\n# a comment\n"),
        )
        .unwrap();
        let run = |lines: &[&str]| env_diff(&env, &v(lines));

        assert_eq!(
            run(&[
                "PATH=/usr/bin",
                "POSTGRES_DB=axon",
                &format!("POSTGRES_PASSWORD={secret}")
            ]),
            (String::new(), true)
        );
        let (out, ok) = run(&["POSTGRES_DB=axon", "POSTGRES_PASSWORD=old-value"]);
        assert!(!ok && out.contains("value differs: POSTGRES_PASSWORD"));
        assert!(
            !out.contains(secret) && !out.contains("old-value"),
            "values never appear"
        );
        let (out, _) = run(&["POSTGRES_DB=axon"]);
        assert!(
            out.contains("declared, not in container: POSTGRES_PASSWORD") && !out.contains(secret)
        );
        assert_eq!(
            run(&[
                "PATH=/x",
                "LANG=C",
                "POSTGRES_DB=axon",
                &format!("POSTGRES_PASSWORD={secret}")
            ])
            .0,
            "",
            "image defaults are not drift"
        );

        let bare = dir.join("env-bare");
        std::fs::write(&bare, "FROM_HOST\n").unwrap();
        assert!(env_diff(&bare, &v(&["FROM_HOST=whatever"]))
            .0
            .contains("cannot check"));
        assert!(env_diff(&dir.join("does-not-exist"), &v(&["X=1"]))
            .0
            .contains("env file is missing"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
