//! Finding, joining and reaching the Mac, and the files a pairing leaves on the phone.

use super::*;

/// A Mac that `host/Discovery.swift` found over Bonjour, from the TXT record the LAN listener
/// advertises (`crates/sjel-status/src/lan.rs`).
#[derive(Clone, Debug, Deserialize)]
pub struct Found {
    /// The Bonjour instance name, "Sjel on <mac>".
    pub name: String,
    /// The Mac's name without `.local`; the listener's certificate carries `<host>.local`.
    pub host: String,
    pub port: u16,
    /// SHA-256 of the listener's certificate, 64 lowercase hex characters.
    pub fp: String,
}

impl Found {
    /// `<mac name> <port> <fingerprint>`, the text `device::parse_join_target` reads.
    pub(crate) fn join_target(&self) -> String {
        format!("{} {} {}", self.host, self.port, self.fp)
    }

    pub(crate) fn address(&self) -> String {
        format!("{}.local:{}", self.host, self.port)
    }
}

/// The saved node, or a new one, then the node's health as this device.
pub(crate) async fn load(
    pair: Option<String>,
    join: Option<String>,
    events: &UnboundedSender<Event>,
) -> Result<(), String> {
    let path = node_file()?;
    let node = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| "The saved pairing is damaged.")?,
        Err(_) => {
            let node = match (pair, join) {
                (Some(pair), _) => {
                    let qr = device::parse_pairing_qr(&pair)?;
                    device::claim(&qr, label(), &identity::public_key()?).await?
                }
                (None, Some(join)) => join_and_wait(&join, events).await?,
                (None, None) => return Err("Not paired with a Mac yet.".into()),
            };
            node
        }
    };
    connect(&path, node, events).await
}

/// Saves the node, then reads the Mac's health as this device. The save comes first so a
/// connection that fails afterwards does not lose a pairing the owner already allowed.
pub(crate) async fn connect(
    path: &Path,
    node: Node,
    events: &UnboundedSender<Event>,
) -> Result<(), String> {
    save(path, &node)?;
    let sign: Signer = Arc::new(identity::sign);
    let client = DeviceClient::new(node, sign)?;
    let health = client.get::<NodeHealth>(HEALTH).await?;
    let _ = events.unbounded_send(Event::Ready(client.node().clone(), health));
    Ok(())
}

/// Asks to join, shows the codes to compare, and waits until the owner decides on the Mac.
pub(crate) async fn join_and_wait(
    target: &str,
    events: &UnboundedSender<Event>,
) -> Result<Node, String> {
    let (base, pin) = device::parse_join_target(target)?;
    let request = device::request_join(&base, &pin, label(), &identity::public_key()?).await?;
    let _ = events.unbounded_send(Event::Status(format!(
        "Allow \"{}\" in Devices on the Mac if it shows {} · {}",
        label(),
        device::comparison_code(&pin),
        request.key_code
    )));
    loop {
        tokio::time::sleep(JOIN_POLL).await;
        let state = device::join_state(&base, &pin, &request.id).await?;
        match (state.status.as_str(), state.device) {
            ("pending", _) => continue,
            ("approved", Some(device)) => {
                return Ok(Node {
                    base,
                    pin,
                    device_id: device.id,
                })
            }
            ("denied", _) => return Err("The Mac denied this device.".into()),
            ("expired", _) => return Err("The request expired. Ask again.".into()),
            (other, _) => return Err(format!("The Mac reported the request as {other}.")),
        }
    }
}

/// Claims a code the camera read: the Mac's address, its certificate pin and a one-time code in
/// one string (`device::PairingQr`). The pin is what makes a scan the trusted channel: the claim
/// goes to the host the code named, over a connection pinned to the certificate it named, so a
/// code read off the wrong screen cannot reach a different machine.
pub(crate) async fn claim_scanned(code: &str) -> Result<Node, String> {
    let qr = device::parse_pairing_qr(code)?;
    device::claim(&qr, label(), &identity::public_key()?).await
}

pub(crate) fn save(path: &std::path::Path, node: &Node) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Save the pairing: {e}"))?;
    }
    let json = serde_json::to_vec(node).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| format!("Save the pairing: {e}"))
}

/// The app container's Application Support, where the pairing (not the key) is kept.
pub(crate) fn node_file() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("No app container (HOME is not set).")?;
    Ok(PathBuf::from(home).join("Library/Application Support/sjel-node.json"))
}

/// Where the Mac's read-only copy lives on this phone: beside the pairing, in the container.
pub(crate) fn copy_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME").ok_or("No app container (HOME is not set).")?;
    Ok(PathBuf::from(home).join("Library/Application Support/sjel-mirror"))
}

/// The copy this phone already holds, read without the Mac. The Calendar page opens on this so a
/// launch on a train shows the last sync's day instead of an empty page (`core/src/mirror.rs`).
pub(crate) fn open_copy() -> Result<Arc<Mirror>, String> {
    let dir = copy_dir()?;
    Mirror::open(&dir).map(Arc::new)
}

/// Asks the Mac to build a fresh copy, then fetches whatever it now has.
pub(crate) async fn fetch_copy(client: Arc<DeviceClient>) -> Result<Arc<Mirror>, String> {
    let dir = copy_dir()?;
    // The rebuild is a request, not a demand: the route sits on the Mac's operator gate, and a
    // device that is refused it still gets the copy the Mac already holds (whose age the page
    // states).
    let _ = mirror::rebuild(&*client).await;
    mirror::refresh(&*client, &dir).await?;
    Mirror::open(&dir).map(Arc::new)
}
