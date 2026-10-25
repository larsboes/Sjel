//! The read-only copy of the Mac's data on the phone, and the fetch that keeps it current.
//!
//! `sjel mirror build` writes the copy on the Mac and `GET /api/sjel-status/mirror` serves it with
//! a manifest beside it (`capabilities/sjel-status/src/status/mirror.rs`, route R1 in
//! `research/offline-on-the-phone.md`). This module is the phone's half: read the manifest, fetch
//! the copy only when its digest differs from the one already on disk, check the bytes against the
//! digest before they replace anything, then read the copy locally.
//!
//! The copy is read-only by intent. It answers reads when the Mac is unreachable and takes no
//! writes, so anything drawn from it is stamped with when it was built rather than pretending to
//! be live. Writes go to the replica instead ([`crate::store`], replayed by [`crate::sync`]),
//! which covers the projects domain today; a page that needs to write asks the replica, and a page
//! that only needs to read can ask the copy.
//!
//! The table set is the Mac's decision, not this module's: the manifest names what it holds
//! (`projects_records`, `calendar_entries`, `calendar_rhythms`, `entities_entities` with
//! `entities_values`, and `places_places` with `places_person_places` today), and a read of a table
//! the copy does not carry answers empty rather than failing, so a page can say which copy it drew
//! instead of blanking.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::places::{PersonLink, Place};
use crate::shell::{CalendarEntry, EntityRecord};
use crate::sync::{Host, Reply};

/// The manifest, which is read first so a digest can be compared before a whole copy travels.
pub const MANIFEST_PATH: &str = "/api/sjel-status/mirror/manifest";
/// The copy itself, served by `ServeFile` so it can be resumed.
pub const COPY_PATH: &str = "/api/sjel-status/mirror";
/// Asks the Mac to build a copy now, and answers the fresh manifest.
pub const REBUILD_PATH: &str = "/api/sjel-status/mirror/rebuild";

/// The copy's file name inside its directory, and the manifest beside it.
const COPY_FILE: &str = "mirror.db";
const MANIFEST_FILE: &str = "manifest.json";
/// Where the day's entries are read from in the copy (`capabilities/calendar/src/store.rs`).
const ENTRIES: &str = "calendar_entries";
/// The contact registry, and the values that make a person a contact card.
const ENTITIES: &str = "entities_entities";
const VALUES: &str = "entities_values";
/// The places register and the links between a place and a person's name.
const PLACES: &str = "places_places";
const PERSON_PLACES: &str = "places_person_places";

/// One table's contribution, as the Mac's manifest reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableCopy {
    pub name: String,
    pub rows: u64,
}

/// What a build produced, written beside the copy as `manifest.json` (`sjel mirror build`).
///
/// Only the fields a reader needs are typed; the rest are kept by `serde`'s default so a manifest
/// from a newer producer still parses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// When the copy was built, e.g. `2026-10-21T21:21:30Z`.
    pub built_at: String,
    /// `ok`, or what `PRAGMA integrity_check` said instead.
    #[serde(default)]
    pub integrity: String,
    /// Foreign keys the selection breaks. Zero for the whole database; a table subset is expected
    /// to break some, because a table whose parent was not taken still declares the reference.
    #[serde(default)]
    pub foreign_key_violations: u64,
    /// The database the copy was taken from.
    #[serde(default)]
    pub source: String,
    /// `phone` or `all`.
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub tables: Vec<TableCopy>,
    pub rows: u64,
    pub bytes: u64,
    pub sha256: String,
    /// Where the copy lives on the Mac. Recorded, never followed: the phone's copy is its own file.
    #[serde(default)]
    pub path: String,
}

impl Manifest {
    /// True when this manifest describes the same bytes as `other`.
    fn same_copy(&self, other: &Self) -> bool {
        self.sha256 == other.sha256 && self.bytes == other.bytes
    }

    /// Whether the copy holds `table`, so a page can say why a read is empty.
    pub fn holds(&self, table: &str) -> bool {
        self.tables.iter().any(|held| held.name == table)
    }

    /// `2026-10-21 21:21 UTC` — the copy's age, for a line that says how fresh what is on screen is.
    pub fn built_label(&self) -> String {
        match self.built_at.get(..16) {
            Some(stamp) => format!("{} UTC", stamp.replace('T', " ")),
            None => self.built_at.clone(),
        }
    }
}

/// What one refresh did.
#[derive(Debug)]
pub enum Refreshed {
    /// The copy on disk is already the Mac's newest, and its manifest.
    Already(Manifest),
    /// The copy was fetched and replaced, and its manifest.
    Downloaded(Manifest),
}

impl Refreshed {
    pub fn manifest(&self) -> &Manifest {
        match self {
            Self::Already(manifest) | Self::Downloaded(manifest) => manifest,
        }
    }

    /// The sentence the phone shows under the page title.
    pub fn summary(&self) -> String {
        match self {
            Self::Already(manifest) => format!("Copy from {}", manifest.built_label()),
            Self::Downloaded(manifest) => {
                format!("Copy fetched · built {}", manifest.built_label())
            }
        }
    }
}

/// The copy's contact registry and places register, read once for one page.
///
/// One read and not three, because a person's card is a join: their own row, the values they carry,
/// and the places the register links to their name. Three opens of one file could answer three
/// different versions of it.
#[derive(Clone, Debug, Default)]
pub struct PeopleCopy {
    /// Everyone of kind `person`, name order, with their values folded in.
    pub people: Vec<EntityRecord>,
    /// The places register, kind then name order.
    pub places: Vec<Place>,
    /// The links the operator has not dismissed, in name order.
    pub links: Vec<PersonLink>,
}

impl PeopleCopy {
    /// Why the list is empty, in the copy's own terms: an older copy simply does not hold these.
    pub fn absence(&self, manifest: &Manifest) -> Option<String> {
        if !manifest.holds(ENTITIES) {
            return Some("This copy was built without the contact registry.".to_owned());
        }
        None
    }
}

/// The copy on disk, opened for reading: its manifest and its path.
#[derive(Debug)]
pub struct Mirror {
    path: PathBuf,
    manifest: Manifest,
}

impl Mirror {
    /// Opens the copy in `dir`, or says why it cannot.
    ///
    /// Nothing is written and nothing is fetched: [`refresh`] does both, and a caller that wants a
    /// copy first calls it. An absent manifest and an absent file are ordinary states on a phone
    /// that has never reached its Mac, so the message names the action that fixes it.
    pub fn open(dir: &Path) -> Result<Self, String> {
        let manifest_path = dir.join(MANIFEST_FILE);
        let body = match std::fs::read_to_string(&manifest_path) {
            Ok(body) => body,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(
                    "No copy on this phone yet. It arrives when this phone reaches your Mac."
                        .to_owned(),
                )
            }
            Err(error) => return Err(format!("The copy's manifest cannot be read: {error}")),
        };
        let manifest: Manifest = serde_json::from_str(&body)
            .map_err(|error| format!("The copy's manifest is unreadable: {error}"))?;
        let path = dir.join(COPY_FILE);
        let bytes = match std::fs::metadata(&path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err("The copy is incomplete on this phone: sync again.".to_owned())
            }
            Err(error) => return Err(format!("The copy's file cannot be read: {error}")),
        };
        if bytes != manifest.bytes {
            return Err(format!(
                "The copy is {} bytes but its manifest says {}: sync again.",
                bytes, manifest.bytes
            ));
        }
        Ok(Self { path, manifest })
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The day's entries: `[from, to)` in the copy's own words, exactly the window the calendar
    /// capability uses (`plain_day_window` in `capabilities/calendar/src/store.rs`).
    ///
    /// A copy taken without `calendar_entries` answers an empty list rather than an error: the
    /// phone is showing a real absence, and the page says which copy it is reading.
    pub async fn calendar(&self, from: &str, to: &str) -> Result<Vec<CalendarEntry>, String> {
        if !self.manifest.holds(ENTRIES) {
            return Ok(Vec::new());
        }
        let db = libsql::Builder::new_local(&self.path)
            .build()
            .await
            .map_err(|error| format!("Cannot open the copy: {error}"))?;
        let conn = db
            .connect()
            .map_err(|error| format!("Cannot read the copy: {error}"))?;
        // `[from, to)`, with the bare `to` date so an all-day entry that ends that morning is in
        // the window and one that starts that morning is not.
        let query = "SELECT title, starts_at, ends_at, all_day, commitment, id, kind, location, \
                     notes, source \
                     FROM calendar_entries \
                     WHERE starts_at < ?1 AND ends_at > ?2 \
                     ORDER BY all_day DESC, starts_at ASC";
        let mut rows = conn
            .query(
                query,
                libsql::params![to.to_owned(), format!("{from}T00:00:00")],
            )
            .await
            .map_err(|error| format!("Cannot read the copy's calendar: {error}"))?;
        let mut list = Vec::new();
        while let Ok(Some(row)) = rows.next().await {
            list.push(CalendarEntry {
                title: row.get(0).unwrap_or_default(),
                starts_at: row.get(1).unwrap_or_default(),
                ends_at: row.get(2).unwrap_or_default(),
                all_day: row.get::<i64>(3).unwrap_or(0) != 0,
                commitment: row.get(4).unwrap_or_else(|_| "possible".into()),
                id: row.get(5).unwrap_or_default(),
                kind: row.get(6).unwrap_or_default(),
                location: row.get(7).ok(),
                notes: row.get(8).ok(),
                source: row.get(9).unwrap_or_default(),
            });
        }
        Ok(list)
    }

    /// The contacts and the places register, as one answer ([`PeopleCopy`]).
    ///
    /// A copy the Mac built before these tables travelled answers empty lists rather than an error:
    /// an older `sjel mirror build` is a real absence, and the page says which copy it drew. A copy
    /// that holds the registry but not its values still answers the names, which is all an older
    /// copy honestly knows.
    pub async fn people(&self) -> Result<PeopleCopy, String> {
        let mut copy = PeopleCopy::default();
        let holds_entities = self.manifest.holds(ENTITIES);
        let holds_places = self.manifest.holds(PLACES);
        if !holds_entities && !holds_places {
            return Ok(copy);
        }
        let db = libsql::Builder::new_local(&self.path)
            .build()
            .await
            .map_err(|error| format!("Cannot open the copy: {error}"))?;
        let conn = db
            .connect()
            .map_err(|error| format!("Cannot read the copy: {error}"))?;

        if holds_entities {
            // A left join, so somebody the operator added and has not filled in yet is still a row.
            // Without `entities_values` there is nothing to join: the page draws names and says so.
            let query = if self.manifest.holds(VALUES) {
                "SELECT e.id, e.kind, e.name, e.note_ref, e.revision, v.key, v.value \
                 FROM entities_entities e \
                 LEFT JOIN entities_values v ON v.entity_id = e.id \
                 WHERE e.kind = 'person' \
                 ORDER BY e.name COLLATE NOCASE, v.key"
            } else {
                "SELECT e.id, e.kind, e.name, e.note_ref, e.revision, NULL, NULL \
                 FROM entities_entities e \
                 WHERE e.kind = 'person' \
                 ORDER BY e.name COLLATE NOCASE"
            };
            let mut rows = conn
                .query(query, ())
                .await
                .map_err(|error| format!("Cannot read the copy's contacts: {error}"))?;
            while let Ok(Some(row)) = rows.next().await {
                let id: String = row.get(0).unwrap_or_default();
                let key: Option<String> = row.get(5).ok();
                let value: Option<String> = row.get(6).ok();
                // One row per value, so a person is as many rows as they have values: the record is
                // created when the id changes and every later row adds to it.
                if copy.people.last().is_none_or(|record| record.id != id) {
                    copy.people.push(EntityRecord {
                        id: id.clone(),
                        kind: row.get(1).unwrap_or_else(|_| "person".to_owned()),
                        name: row.get(2).unwrap_or_default(),
                        note_ref: row.get(3).ok(),
                        revision: row.get::<i64>(4).unwrap_or(0).clamp(0, u32::MAX as i64) as u32,
                        values: Default::default(),
                        // The copy holds no facts table; the Mac serves them live.
                        facts: Vec::new(),
                    });
                }
                if let (Some(key), Some(value)) = (key, value) {
                    if let Some(record) = copy.people.last_mut() {
                        record.values.insert(key, json_value(&value));
                    }
                }
            }
        }

        if holds_places {
            let mut rows = conn
                .query(
                    "SELECT id, name, kind, city, country_code, address, latitude, longitude \
                     FROM places_places ORDER BY kind, name COLLATE NOCASE",
                    (),
                )
                .await
                .map_err(|error| format!("Cannot read the copy's places: {error}"))?;
            while let Ok(Some(row)) = rows.next().await {
                copy.places.push(Place {
                    id: row.get(0).unwrap_or_default(),
                    name: row.get(1).unwrap_or_default(),
                    kind: row.get(2).unwrap_or_else(|_| "venue".to_owned()),
                    city: row.get(3).ok(),
                    country_code: row.get(4).ok(),
                    address: row.get(5).ok(),
                    latitude: row.get(6).ok(),
                    longitude: row.get(7).ok(),
                });
            }

            // A dismissed row is not a link: the operator already said that place does not belong
            // to that person, and the register never offers it again.
            if self.manifest.holds(PERSON_PLACES) {
                let mut rows = conn
                    .query(
                        "SELECT person, place_id, state FROM places_person_places \
                         WHERE state <> 'dismissed' ORDER BY person COLLATE NOCASE, state",
                        (),
                    )
                    .await
                    .map_err(|error| format!("Cannot read the copy's person links: {error}"))?;
                while let Ok(Some(row)) = rows.next().await {
                    copy.links.push(PersonLink {
                        person: row.get(0).unwrap_or_default(),
                        place_id: row.get(1).unwrap_or_default(),
                        state: row.get(2).unwrap_or_else(|_| "proposed".to_owned()),
                    });
                }
            }
        }
        Ok(copy)
    }
}

/// One value as the registry stores it: the text of a JSON value (`capabilities/entities/src/store.rs`
/// writes `value.to_string()`). A row that does not parse is kept as the text it is, so an
/// unfamiliar value shows up in the card instead of vanishing from it.
fn json_value(text: &str) -> serde_json::Value {
    serde_json::from_str(text).unwrap_or_else(|_| serde_json::Value::String(text.to_owned()))
}

/// Fetches the Mac's manifest and, when its digest differs from the copy on disk, the copy too.
///
/// The order is the safety property: the manifest travels first so a copy already current costs
/// one small answer, the bytes are checked against the manifest's digest before they are written,
/// and the new copy is renamed into place, so a refresh that dies halfway leaves the previous copy
/// whole rather than a file that is half of each.
pub async fn refresh(host: &impl Host, dir: &Path) -> Result<Refreshed, String> {
    let manifest = fetch_manifest(host).await?;
    if let Ok(current) = Mirror::open(dir) {
        if current.manifest.same_copy(&manifest) {
            return Ok(Refreshed::Already(manifest));
        }
    }
    let bytes = fetch_copy(host, &manifest).await?;
    let copy = write_copy(dir, &bytes)?;
    match write_manifest(dir, &manifest) {
        Ok(()) => Ok(Refreshed::Downloaded(manifest)),
        Err(error) => {
            // Without the matching manifest the copy cannot be told apart from a stale one, so it
            // is removed rather than left to be read as current by a later open.
            let _ = std::fs::remove_file(&copy);
            Err(error)
        }
    }
}

/// Asks the Mac to rebuild its copy and answers the manifest it produced.
///
/// This is a request, not a demand: the route runs `sjel mirror build` on the operator surface and
/// a paired device can be refused it (`capabilities/sjel-status/src/status/mirror.rs`). A caller
/// that is refused still has [`refresh`], which takes whatever copy the Mac already holds.
pub async fn rebuild(host: &impl Host) -> Result<Manifest, String> {
    let reply = host.send("POST", REBUILD_PATH, Some(b"{}")).await?;
    if reply.status != 200 {
        return Err(refusal(&reply, "a fresh copy"));
    }
    serde_json::from_slice(&reply.body)
        .map_err(|error| format!("The Mac's manifest is not readable: {error}"))
}

async fn fetch_manifest(host: &impl Host) -> Result<Manifest, String> {
    let reply = host.send("GET", MANIFEST_PATH, None).await?;
    if reply.status != 200 {
        return Err(refusal(&reply, "its manifest"));
    }
    serde_json::from_slice(&reply.body)
        .map_err(|error| format!("The Mac's manifest is not readable: {error}"))
}

async fn fetch_copy(host: &impl Host, manifest: &Manifest) -> Result<Vec<u8>, String> {
    let reply = host.send("GET", COPY_PATH, None).await?;
    if reply.status != 200 {
        return Err(refusal(&reply, "its copy"));
    }
    if reply.body.len() as u64 != manifest.bytes {
        return Err(format!(
            "The copy arrived with {} bytes, and the manifest says {}.",
            reply.body.len(),
            manifest.bytes
        ));
    }
    let digest = sha256_of(&reply.body);
    if digest != manifest.sha256 {
        return Err(format!(
            "The copy arrived with digest {digest}, not the {} its manifest named.",
            manifest.sha256
        ));
    }
    Ok(reply.body)
}

/// What the Mac said when it refused, in its own words: `{"error": "…"}` is how the status
/// capability reports an unbuilt or unreadable copy, and that sentence names the fixing action.
fn refusal(reply: &Reply, what: &str) -> String {
    serde_json::from_slice::<serde_json::Value>(&reply.body)
        .ok()
        .and_then(|body| body["error"].as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("the Mac answered {} for {what}", reply.status))
}

/// Writes the copy beside its destination and renames it, so no reader sees a partial file.
fn write_copy(dir: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|error| format!("Cannot keep a copy: {error}"))?;
    let staged = dir.join(format!("{COPY_FILE}.part"));
    let target = dir.join(COPY_FILE);
    std::fs::write(&staged, bytes).map_err(|error| format!("Cannot keep a copy: {error}"))?;
    std::fs::rename(&staged, &target).map_err(|error| {
        let _ = std::fs::remove_file(&staged);
        format!("Cannot keep a copy: {error}")
    })?;
    Ok(target)
}

fn write_manifest(dir: &Path, manifest: &Manifest) -> Result<(), String> {
    let staged = dir.join(format!("{MANIFEST_FILE}.part"));
    let target = dir.join(MANIFEST_FILE);
    let body = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("Cannot record the manifest: {error}"))?;
    std::fs::write(&staged, body)
        .map_err(|error| format!("Cannot record the manifest: {error}"))?;
    std::fs::rename(&staged, &target).map_err(|error| {
        let _ = std::fs::remove_file(&staged);
        format!("Cannot record the manifest: {error}")
    })
}

fn sha256_of(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A Mac that answers from a table, one answer per path, so a test states what it expects to
    /// be asked. A request nobody queued fails loudly instead of answering empty.
    #[derive(Default)]
    struct Fake {
        answers: Mutex<Vec<(String, u16, Vec<u8>)>>,
        seen: Mutex<Vec<String>>,
    }

    impl Fake {
        fn on(&self, path: &str, status: u16, body: impl Into<Vec<u8>>) {
            self.answers
                .lock()
                .unwrap()
                .push((path.to_owned(), status, body.into()));
        }

        fn asked(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl Host for Fake {
        async fn send(
            &self,
            _method: &str,
            path: &str,
            _body: Option<&[u8]>,
        ) -> Result<Reply, String> {
            self.seen.lock().unwrap().push(path.to_owned());
            let mut answers = self.answers.lock().unwrap();
            let index = answers
                .iter()
                .position(|(answered, _, _)| answered == path)
                .ok_or_else(|| format!("nothing queued for {path}"))?;
            let (_, status, body) = answers.remove(index);
            Ok(Reply { status, body })
        }
    }

    /// A directory of this test's own: the process, the clock and a counter, because these tests
    /// run on parallel threads and two of them must never open one file.
    fn dir() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let count = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sjel-mirror-{}-{unique}-{count}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// A copy holding one day, written the way the Mac writes it: a plain SQLite file.
    async fn copy_bytes(dir: &Path) -> Vec<u8> {
        let path = dir.join("built.db");
        let db = libsql::Builder::new_local(&path)
            .build()
            .await
            .expect("build a copy");
        let conn = db.connect().expect("connect");
        conn.execute_batch("PRAGMA busy_timeout = 5000;")
            .await
            .expect("a busy timeout");
        conn.execute_batch(
            "CREATE TABLE calendar_entries (
                 id TEXT PRIMARY KEY, kind TEXT NOT NULL,
                 commitment TEXT NOT NULL DEFAULT 'possible', title TEXT NOT NULL,
                 starts_at TEXT NOT NULL, ends_at TEXT NOT NULL,
                 all_day INTEGER NOT NULL DEFAULT 0, location TEXT, notes TEXT,
                 attendees TEXT NOT NULL DEFAULT '[]', source TEXT NOT NULL DEFAULT 'manual',
                 external_id TEXT, rhythm_id TEXT, payload TEXT NOT NULL DEFAULT '{}',
                 created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
             INSERT INTO calendar_entries
                 (id, kind, commitment, title, starts_at, ends_at, all_day, location, source, created_at, updated_at)
             VALUES
                 ('e1', 'work_onsite', 'committed', 'Standup', '2026-10-23T09:30:00',
                  '2026-10-23T09:45:00', 0, 'Room 2', 'manual', '2026-10-20', '2026-10-20'),
                 ('e2', 'away', 'planned', 'Reykjavík', '2026-10-23', '2026-10-24',
                  1, NULL, 'manual', '2026-10-20', '2026-10-20');",
        )
        .await
        .expect("a table with two entries");
        // The connection is closed before the file is read: the copy travels as bytes, and an open
        // handle on this machine is a lock the fetch path would never hold.
        drop(conn);
        drop(db);
        std::fs::read(&path).expect("read the copy")
    }

    #[test]
    fn a_copy_answers_a_contact_card_and_the_places_it_links_to() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let dir = dir();
            // A copy built by a Mac that holds the contact and place tables, in the Mac's own
            // schema: values as JSON text, the person's name on the link and not their id.
            let path = dir.join("built.db");
            let db = libsql::Builder::new_local(&path)
                .build()
                .await
                .expect("build a copy");
            let conn = db.connect().expect("connect");
            conn.execute_batch(
                "CREATE TABLE entities_entities (id TEXT PRIMARY KEY, kind TEXT NOT NULL,
                     name TEXT NOT NULL, note_ref TEXT, revision INTEGER NOT NULL,
                     created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
                 CREATE TABLE entities_values (entity_id TEXT NOT NULL, key TEXT NOT NULL,
                     value TEXT NOT NULL, source TEXT NOT NULL, updated_at TEXT NOT NULL,
                     PRIMARY KEY (entity_id, key));
                 CREATE TABLE places_places (id TEXT PRIMARY KEY, name TEXT NOT NULL,
                     kind TEXT NOT NULL, address TEXT, city TEXT, country_code TEXT,
                     latitude REAL, longitude REAL, source TEXT NOT NULL, external_ref TEXT,
                     created_at TEXT NOT NULL);
                 CREATE TABLE places_person_places (id TEXT PRIMARY KEY, person TEXT NOT NULL,
                     place_id TEXT NOT NULL, date_start TEXT, date_end TEXT,
                     confidence_bp INTEGER NOT NULL, source TEXT NOT NULL, state TEXT NOT NULL,
                     created_at TEXT NOT NULL, reviewed_at TEXT);
                 INSERT INTO entities_entities VALUES
                     ('ent:1', 'person', 'Ada Lovelace', 'Atlas/People/Ada.md', 4,
                      '2026-01-01', '2026-10-01'),
                     ('ent:2', 'person', 'Alan Turing', NULL, 1, '2026-01-01', '2026-01-01');
                 INSERT INTO entities_values VALUES
                     ('ent:1', 'emails', '[\"ada@example.com\"]', 'google', '2026-10-01'),
                     ('ent:1', 'phones', '[\"+44 20 7946 0958\"]', 'google', '2026-10-01'),
                     ('ent:1', 'company', '\"Analytical Engine Inc\"', 'operator', '2026-10-01');
                 INSERT INTO places_places VALUES
                     ('pl:1', 'Konzerthaus', 'venue', 'Gendarmenmarkt 2', 'Berlin', 'DE',
                      52.5138, 13.3922, 'operator', NULL, '2026-01-01'),
                     ('pl:2', 'Zürich', 'city', NULL, 'Zürich', 'CH', NULL, NULL,
                      'operator', NULL, '2026-01-01');
                 INSERT INTO places_person_places VALUES
                     ('pp:1', 'Ada Lovelace', 'pl:1', NULL, NULL, 9000, 'operator',
                      'confirmed', '2026-01-01', NULL),
                     ('pp:2', 'Alan Turing', 'pl:2', NULL, NULL, 4000, 'operator',
                      'dismissed', '2026-01-01', NULL);",
            )
            .await
            .expect("a copy with contacts");
            drop(conn);
            drop(db);
            let bytes = std::fs::read(&path).expect("read the copy");
            let mut manifest = manifest_for(&bytes);
            manifest.tables = vec![
                TableCopy {
                    name: ENTITIES.to_owned(),
                    rows: 2,
                },
                TableCopy {
                    name: VALUES.to_owned(),
                    rows: 3,
                },
                TableCopy {
                    name: PLACES.to_owned(),
                    rows: 2,
                },
                TableCopy {
                    name: PERSON_PLACES.to_owned(),
                    rows: 2,
                },
            ];
            std::fs::write(
                dir.join(MANIFEST_FILE),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .expect("manifest");
            std::fs::write(dir.join(COPY_FILE), &bytes).expect("copy");

            let mirror = Mirror::open(&dir).expect("the copy opens");
            let copy = mirror.people().await.expect("read the contacts");
            assert_eq!(copy.people.len(), 2, "one row per person, not per value");
            let ada = copy
                .people
                .iter()
                .find(|record| record.name == "Ada Lovelace")
                .expect("Ada");
            assert_eq!(ada.revision, 4);
            assert_eq!(ada.note_ref.as_deref(), Some("Atlas/People/Ada.md"));
            // The value is JSON text in the table and a value again here.
            assert_eq!(ada.values["emails"], serde_json::json!(["ada@example.com"]));

            let person = crate::people::Person::from_record(ada);
            assert_eq!(person.initials, "AL");
            assert_eq!(person.email.as_deref(), Some("ada@example.com"));
            assert_eq!(person.phone.as_deref(), Some("+44 20 7946 0958"));
            assert_eq!(person.company.as_deref(), Some("Analytical Engine Inc"));

            // The dismissed link is not in the copy's answer at all.
            assert_eq!(copy.links.len(), 1);
            let places = crate::places::places_of(&copy.places, &copy.links, &person.name);
            assert_eq!(
                places
                    .iter()
                    .map(|place| place.name.as_str())
                    .collect::<Vec<_>>(),
                vec!["Konzerthaus"]
            );
            assert_eq!(places[0].where_line(), "Berlin, DE");
        });
    }

    #[test]
    fn a_copy_without_the_contacts_answers_none_rather_than_failing() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let dir = dir();
            let bytes = copy_bytes(&dir).await;
            let manifest = manifest_for(&bytes);
            std::fs::write(
                dir.join(MANIFEST_FILE),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .expect("manifest");
            std::fs::write(dir.join(COPY_FILE), &bytes).expect("copy");

            let mirror = Mirror::open(&dir).expect("the copy opens");
            let copy = mirror
                .people()
                .await
                .expect("an older copy is not an error");
            assert!(copy.people.is_empty());
            assert!(copy.places.is_empty());
            // Both absences are answers the page can say out loud.
            assert!(!mirror.manifest().holds(ENTITIES));
            assert!(copy.absence(mirror.manifest()).is_some());
        });
    }

    fn manifest_for(bytes: &[u8]) -> Manifest {
        Manifest {
            built_at: "2026-10-21T21:21:30Z".to_owned(),
            integrity: "ok".to_owned(),
            foreign_key_violations: 0,
            source: "/Users/someone/overlay/data/axon/axon.db".to_owned(),
            mode: "phone".to_owned(),
            tables: vec![
                TableCopy {
                    name: "projects_records".to_owned(),
                    rows: 1221,
                },
                TableCopy {
                    name: ENTRIES.to_owned(),
                    rows: 2,
                },
            ],
            rows: 1223,
            bytes: bytes.len() as u64,
            sha256: sha256_of(bytes),
            path: "/Users/someone/overlay/data/mirror/mirror.db".to_owned(),
        }
    }

    #[test]
    fn a_copy_that_is_already_current_is_not_fetched_again() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let dir = dir();
            let bytes = copy_bytes(&dir).await;
            let manifest = manifest_for(&bytes);
            std::fs::write(
                dir.join(MANIFEST_FILE),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .expect("manifest");
            std::fs::write(dir.join(COPY_FILE), &bytes).expect("copy");

            let fake = Fake::default();
            fake.on(MANIFEST_PATH, 200, serde_json::to_vec(&manifest).unwrap());
            let outcome = refresh(&fake, &dir).await.expect("refresh");
            assert!(matches!(outcome, Refreshed::Already(_)));
            // The copy was never asked for, and nothing on disk moved.
            assert_eq!(fake.asked(), vec![MANIFEST_PATH.to_owned()]);
            assert_eq!(std::fs::read(dir.join(COPY_FILE)).unwrap(), bytes);
        });
    }

    #[test]
    fn a_newer_copy_replaces_the_old_one_and_its_entries_are_readable() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let dir = dir();
            let bytes = copy_bytes(&dir).await;
            let manifest = manifest_for(&bytes);
            let fake = Fake::default();
            fake.on(MANIFEST_PATH, 200, serde_json::to_vec(&manifest).unwrap());
            fake.on(COPY_PATH, 200, bytes.clone());

            let outcome = refresh(&fake, &dir).await.expect("refresh");
            assert!(matches!(outcome, Refreshed::Downloaded(_)));
            assert_eq!(
                fake.asked(),
                vec![MANIFEST_PATH.to_owned(), COPY_PATH.to_owned()]
            );

            let mirror = Mirror::open(&dir).expect("the copy opens");
            assert_eq!(mirror.manifest().built_at, "2026-10-21T21:21:30Z");
            let entries = mirror
                .calendar("2026-10-23", "2026-10-24")
                .await
                .expect("read the day");
            // All-day first, then by start: the trip is the whole day, the standup sits in it.
            assert_eq!(
                entries.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
                vec!["Reykjavík", "Standup"]
            );
            assert!(entries[0].all_day);
            assert_eq!(entries[1].kind, "work_onsite");
            assert_eq!(entries[1].commitment, "committed");
            assert_eq!(entries[1].location.as_deref(), Some("Room 2"));
        });
    }

    #[test]
    fn a_copy_whose_digest_does_not_match_its_manifest_is_refused_and_the_old_copy_stays() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let dir = dir();
            let bytes = copy_bytes(&dir).await;
            let mut manifest = manifest_for(&bytes);
            // The Mac built the copy again after answering the manifest: the bytes it sends are
            // not the bytes the manifest describes.
            let mut tampered = bytes.clone();
            let last = tampered.len() - 1;
            tampered[last] ^= 0xff;
            manifest.bytes = tampered.len() as u64;

            let fake = Fake::default();
            fake.on(MANIFEST_PATH, 200, serde_json::to_vec(&manifest).unwrap());
            fake.on(COPY_PATH, 200, tampered);
            let error = refresh(&fake, &dir)
                .await
                .expect_err("a bad copy is refused");
            assert!(error.contains("digest"), "{error}");
            assert!(!dir.join(COPY_FILE).exists());
            assert!(!dir.join(format!("{MANIFEST_FILE}.part")).exists());
        });
    }

    #[test]
    fn a_manifest_that_names_bytes_the_copy_did_not_deliver_is_refused() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let dir = dir();
            let bytes = copy_bytes(&dir).await;
            let mut manifest = manifest_for(&bytes);
            manifest.bytes += 1;
            let fake = Fake::default();
            fake.on(MANIFEST_PATH, 200, serde_json::to_vec(&manifest).unwrap());
            fake.on(COPY_PATH, 200, bytes);
            let error = refresh(&fake, &dir)
                .await
                .expect_err("a short copy is refused");
            assert!(error.contains("bytes"), "{error}");
        });
    }

    #[test]
    fn an_unbuilt_mac_says_which_command_builds_the_copy() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let dir = dir();
            let fake = Fake::default();
            fake.on(
                MANIFEST_PATH,
                404,
                serde_json::json!({
                    "error": "no copy has been built yet — run `sjel mirror build`"
                })
                .to_string(),
            );
            let error = refresh(&fake, &dir).await.expect_err("no copy is an error");
            assert!(error.contains("sjel mirror build"), "{error}");
        });
    }

    #[test]
    fn a_copy_the_mac_took_without_the_calendar_answers_no_entries() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let dir = dir();
            let bytes = copy_bytes(&dir).await;
            let mut manifest = manifest_for(&bytes);
            manifest.tables.retain(|held| held.name != ENTRIES);
            std::fs::write(
                dir.join(MANIFEST_FILE),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .expect("manifest");
            std::fs::write(dir.join(COPY_FILE), &bytes).expect("copy");

            let mirror = Mirror::open(&dir).expect("the copy opens");
            assert!(!mirror.manifest().holds(ENTRIES));
            assert!(mirror
                .calendar("2026-10-23", "2026-10-24")
                .await
                .unwrap()
                .is_empty());
        });
    }

    #[test]
    fn a_phone_that_never_synced_is_told_when_a_copy_arrives() {
        let dir = dir();
        let error = Mirror::open(&dir).expect_err("nothing to open");
        assert!(error.contains("reaches your Mac"), "{error}");
    }

    #[test]
    fn the_built_label_names_the_day_and_the_hour() {
        let bytes = vec![0u8; 4];
        let manifest = manifest_for(&bytes);
        assert_eq!(manifest.built_label(), "2026-10-21 21:21 UTC");
    }
}
