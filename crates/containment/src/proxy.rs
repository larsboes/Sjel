use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;

/// Hosts permitted for package registries across standard toolchains.
pub const STANDARD_PACKAGE_REGISTRIES: &[&str] = &[
    "crates.io",
    "static.crates.io",
    "index.crates.io",
    "registry.npmjs.org",
    "registry.yarnpkg.com",
    "pypi.org",
    "files.pythonhosted.org",
    "proxy.golang.org",
    "sum.golang.org",
    "github.com",
    "api.github.com",
    "objects.githubusercontent.com",
    "raw.githubusercontent.com",
];

#[derive(Debug, Default, Clone)]
pub struct ProxyAudit {
    pub allowed_count: Arc<AtomicUsize>,
    pub refused_count: Arc<AtomicUsize>,
    pub refusals: Arc<RwLock<Vec<String>>>,
}

#[derive(Debug, Clone)]
pub struct EgressPolicy {
    pub allowed_hosts: HashSet<String>,
    pub audit: ProxyAudit,
}

impl EgressPolicy {
    pub fn new() -> Self {
        let mut allowed = HashSet::new();
        for host in STANDARD_PACKAGE_REGISTRIES {
            allowed.insert((*host).to_lowercase());
        }
        allowed.insert("127.0.0.1".into());
        allowed.insert("localhost".into());
        Self {
            allowed_hosts: allowed,
            audit: ProxyAudit::default(),
        }
    }

    pub fn with_provider_host(mut self, host: impl Into<String>) -> Self {
        self.allowed_hosts.insert(host.into().to_lowercase());
        self
    }

    pub fn is_host_allowed(&self, host: &str) -> bool {
        let host_lower = host.to_lowercase();
        let domain = host_lower.split(':').next().unwrap_or(&host_lower);
        if self.allowed_hosts.contains(domain) {
            return true;
        }
        // Check wildcard subdomain match if applicable
        for allowed in &self.allowed_hosts {
            if domain.ends_with(&format!(".{allowed}")) {
                return true;
            }
        }
        false
    }
}

pub struct EgressProxy {
    listener: TcpListener,
    policy: EgressPolicy,
    local_addr: SocketAddr,
}

impl EgressProxy {
    pub async fn bind(policy: EgressPolicy) -> Result<Self, std::io::Error> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let local_addr = listener.local_addr()?;
        Ok(Self {
            listener,
            policy,
            local_addr,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn audit(&self) -> ProxyAudit {
        self.policy.audit.clone()
    }

    pub async fn run(self) -> Result<(), std::io::Error> {
        loop {
            let (client, _) = self.listener.accept().await?;
            let policy = self.policy.clone();
            tokio::spawn(async move {
                let _ = handle_client(client, policy).await;
            });
        }
    }
}

async fn handle_client(mut client: TcpStream, policy: EgressPolicy) -> Result<(), std::io::Error> {
    let mut buf = [0u8; 4096];
    let n = client.peek(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }

    let request_head = String::from_utf8_lossy(&buf[..n]);
    let first_line = request_head.lines().next().unwrap_or_default();
    let parts: Vec<&str> = first_line.split_whitespace().collect();

    if parts.len() >= 2 && parts[0].eq_ignore_ascii_case("CONNECT") {
        // Read actual request to consume it from stream
        let mut discard = vec![0u8; first_line.len() + 4];
        let _ = client.read(&mut discard).await;

        let target = parts[1];
        let host = target.split(':').next().unwrap_or(target);

        if !policy.is_host_allowed(host) {
            policy.audit.refused_count.fetch_add(1, Ordering::SeqCst);
            policy.audit.refusals.write().await.push(host.to_string());
            let response = b"HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nEgress denied by Sjel containment policy.\r\n";
            client.write_all(response).await?;
            return Ok(());
        }

        policy.audit.allowed_count.fetch_add(1, Ordering::SeqCst);
        let mut target_stream = match TcpStream::connect(target).await {
            Ok(s) => s,
            Err(e) => {
                let response = format!("HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{e}\r\n");
                client.write_all(response.as_bytes()).await?;
                return Ok(());
            }
        };

        client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await?;
        let _ = tokio::io::copy_bidirectional(&mut client, &mut target_stream).await;
    } else {
        // Plain HTTP proxy request
        let target_host = request_head
            .lines()
            .find(|l| l.to_lowercase().starts_with("host:"))
            .and_then(|l| l.split(':').nth(1))
            .map(str::trim)
            .unwrap_or_default();

        if !policy.is_host_allowed(target_host) {
            policy.audit.refused_count.fetch_add(1, Ordering::SeqCst);
            policy.audit.refusals.write().await.push(target_host.to_string());
            let response = b"HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nEgress denied by Sjel containment policy.\r\n";
            client.write_all(response).await?;
            return Ok(());
        }

        policy.audit.allowed_count.fetch_add(1, Ordering::SeqCst);
        let port = 80;
        let connect_target = format!("{target_host}:{port}");
        if let Ok(mut target_stream) = TcpStream::connect(&connect_target).await {
            let _ = tokio::io::copy_bidirectional(&mut client, &mut target_stream).await;
        }
    }

    Ok(())
}
