//! What a RoomPlan capture holds, and how it compares with the tape measure.
//!
//! The raw USDZ carries everything needed: each `WallN.usda` has a local frame in metres plus a
//! `matrix4d xformOp:transform`, and the `Door*` and `Window*` files carry the same. What the
//! phone's semantic draft adds is Apple's per-object confidence, which the USDZ lacks.
//!
//! Metres are allowed here, as the one exception to `model.rs`: the target format
//! `roomplan-capture/v1` names its fields `dimensions_m` and its units `"meters"`. Conversion
//! happens at one boundary, in [`Diff`], where the scan meets `room.toml`.
//!
//! No decompressor: Apple stores USDZ entries with method 0 (`Stored`; all 28 entries checked
//! 2026-09-26). An entry with any other method is an error, not a silently wrong answer.
//!
//! What the capture does not give:
//! - Confidence. Every element goes into the draft as `"low"`; a scan is evidence, not a
//!   measurement.
//! - Room membership. `CapturedRoom.usda` lists room groups with centres but no members, so no
//!   element can be assigned to a room.
//! - Wall thickness as a measurement. All walls carry the same value, a model constant, so it
//!   stays out of the comparison; [`Scan::wall_thickness_m`] only shows it.
//!
//! Nothing is written back. [`Diff`] is a report; the draft can go next to the capture as
//! `draft.json`, where `/api/roomplan/revisions` expects it, and the decision falls in review.

use crate::model::{Model, Room};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// ZIP method 0. Anything else this module cannot read, and says so.
const METHOD_STORED: u16 = 0;

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("{path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path} is not a readable ZIP: {detail}")]
    NotZip { path: PathBuf, detail: String },
    #[error(
        "{path}: `{name}` uses method {method}; this module does not decompress, \
         and a half-read scan is worse than none"
    )]
    Compressed {
        path: PathBuf,
        name: String,
        method: u16,
    },
    #[error("{path}: `{name}` is missing from the archive")]
    Missing { path: PathBuf, name: String },
    #[error("{path}: `{name}` is not readable USDA: {detail}")]
    Form {
        path: PathBuf,
        name: String,
        detail: String,
    },
    #[error("no capture directory: {0}")]
    NoCapture(String),
    #[error("{0}")]
    Assets(String),
}

/// One element of the capture, in the shape `roomplan-capture/v1` requires.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    /// The file name without extension: `Wall0`, `Door1`, `Bed0`.
    pub id: String,
    /// Apple's category, verbatim: `Wall`, `Door(Isopen: False)`, `Window`, `Bed`, `Storage`, …
    pub category: String,
    /// Extent in the local frame, in metres. For walls and objects `y` is the height; the floor
    /// plate is the exception, where `z` is the thickness.
    pub dimensions_m: [f64; 3],
    /// 16 numbers in the order `dashboard/src/lib/roomplan.ts` reads them: the translation is at
    /// `[12..15]`. USD writes rows with the translation in the fourth row, the same order; [`transform`]
    /// explains why nothing is transposed.
    pub transform: [f64; 16],
    /// Always `"low"`: the USDZ carries no confidence.
    pub confidence: &'static str,
    /// The native UUID from `customData`; the capture reports `native_identifiers_present: true`.
    pub source_id: String,
}

impl Element {
    /// Where the element stands in the capture, in metres, from the column the client reads.
    pub fn position_m(&self) -> [f64; 3] {
        [self.transform[12], self.transform[13], self.transform[14]]
    }

    pub fn is_surface(&self) -> bool {
        matches!(self.category.as_str(), "Wall" | "Floor")
    }

    /// Apple's door category carries the state (`Door(Isopen: False)`), so the prefix is checked.
    pub fn is_opening(&self) -> bool {
        self.category.starts_with("Door")
            || self.category.starts_with("Window")
            || self.category.starts_with("Opening")
    }

    pub fn is_door(&self) -> bool {
        self.category.starts_with("Door") || self.category.starts_with("Opening")
    }

    /// Everything that is neither surface nor opening: furniture and appliances.
    pub fn is_object(&self) -> bool {
        !self.is_surface() && !self.is_opening()
    }
}

/// A read capture.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scan {
    pub elements: Vec<Element>,
    /// When the archive was written, from the ZIP timestamp, not now: the scan dates itself.
    pub capture_finished_at: Option<String>,
    /// Number of entries under `assets/Mesh/`.
    pub mesh_assets: usize,
    /// How many room groups `CapturedRoom.usda` lists, without members.
    pub room_groups: usize,
    pub sha256: String,
    pub byte_length: usize,
    /// The floor plate's USDA text: the summary has only spans, and an L-shape's area is not its
    /// span.
    pub boden_usda: Option<String>,
}

impl Scan {
    pub fn surfaces(&self) -> impl Iterator<Item = &Element> {
        self.elements.iter().filter(|e| e.is_surface())
    }
    pub fn openings(&self) -> impl Iterator<Item = &Element> {
        self.elements.iter().filter(|e| e.is_opening())
    }
    pub fn objects(&self) -> impl Iterator<Item = &Element> {
        self.elements.iter().filter(|e| e.is_object())
    }

    pub fn categories(&self) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        for e in &self.elements {
            *out.entry(e.category.clone()).or_insert(0) += 1;
        }
        out
    }

    fn uniform(&self, axis: usize, wanted_category: &str) -> Option<f64> {
        let values: Vec<f64> = self
            .surfaces()
            .filter(|e| e.category == wanted_category)
            .map(|e| e.dimensions_m[axis])
            .collect();
        let first_value = *values.first()?;
        // A micrometre, not equality: mesh output carries values like -0.15999998 next to 0.16.
        if values.iter().all(|w| (w - first_value).abs() < 1e-6) {
            Some(first_value)
        } else {
            None
        }
    }

    /// The height every wall carries, in metres; `None` when the walls disagree.
    pub fn wall_height_m(&self) -> Option<f64> {
        self.uniform(1, "Wall")
    }

    /// The wall thickness in metres when all walls agree, else `None`.
    pub fn wall_thickness_m(&self) -> Option<f64> {
        self.uniform(2, "Wall")
    }

    /// Area of the floor's upper side in square metres.
    pub fn floor_area_m2(&self) -> Option<f64> {
        top_area_m2(self.boden_usda.as_deref()?)
    }
}

// ------------------------------------------------------------------ ZIP

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// A ZIP entry's timestamp as ISO 8601 without a zone. ZIP stores MS-DOS time, the capturing
/// device's clock with no zone field, so none is invented.
fn dos_time(datum: u16, time: u16) -> Option<String> {
    let year = 1980 + ((datum >> 9) & 0x7f) as i64;
    let month = ((datum >> 5) & 0x0f) as u32;
    let tag = (datum & 0x1f) as u32;
    let hour = (time >> 11) as u32;
    let minute = ((time >> 5) & 0x3f) as u32;
    let second = ((time & 0x1f) * 2) as u32;
    if !(1..=12).contains(&month) || !(1..=31).contains(&tag) {
        return None;
    }
    Some(format!(
        "{year:04}-{month:02}-{tag:02}T{hour:02}:{minute:02}:{second:02}"
    ))
}

struct Entry<'a> {
    name: String,
    data: &'a [u8],
    time: Option<String>,
}

/// Reads a ZIP's entries, provided all are stored. Reads the central directory, not the first
/// local header: only there are sizes reliable, since a local header may defer them to a data
/// descriptor.
fn zip_entries<'a>(bytes: &'a [u8], path: &Path) -> Result<Vec<Entry<'a>>, ScanError> {
    let not_zip = |detail: &str| ScanError::NotZip {
        path: path.to_path_buf(),
        detail: detail.to_string(),
    };
    // The EOCD sits at the end, behind a comment of at most 65535 bytes.
    let search_from = bytes.len().saturating_sub(65_557);
    let eocd = (search_from..bytes.len().saturating_sub(3))
        .rev()
        .find(|&i| u32_at(bytes, i) == 0x0605_4b50)
        .ok_or_else(|| not_zip("no end-of-central-directory found"))?;
    let entry_count = u16_at(bytes, eocd + 10) as usize;
    let mut pos = u32_at(bytes, eocd + 16) as usize;

    let mut out = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        if pos + 46 > bytes.len() || u32_at(bytes, pos) != 0x0201_4b50 {
            return Err(not_zip("zentrales Verzeichnis endet vorzeitig"));
        }
        let method = u16_at(bytes, pos + 10);
        let time_raw = u16_at(bytes, pos + 12);
        let date_raw = u16_at(bytes, pos + 14);
        let size = u32_at(bytes, pos + 24) as usize;
        let name_len = u16_at(bytes, pos + 28) as usize;
        let extra_len = u16_at(bytes, pos + 30) as usize;
        let comment_len = u16_at(bytes, pos + 32) as usize;
        let local = u32_at(bytes, pos + 42) as usize;
        let name = String::from_utf8_lossy(&bytes[pos + 46..pos + 46 + name_len]).into_owned();
        pos += 46 + name_len + extra_len + comment_len;

        if name.ends_with('/') {
            continue;
        }
        if method != METHOD_STORED {
            return Err(ScanError::Compressed {
                path: path.to_path_buf(),
                name,
                method,
            });
        }
        // The local header carries its own name and extra lengths, which may differ from the central
        // directory's.
        if local + 30 > bytes.len() || u32_at(bytes, local) != 0x0403_4b50 {
            return Err(not_zip("local header missing"));
        }
        let l_name = u16_at(bytes, local + 26) as usize;
        let l_extra = u16_at(bytes, local + 28) as usize;
        let start = local + 30 + l_name + l_extra;
        if start + size > bytes.len() {
            return Err(not_zip("entry reaches past the archive"));
        }
        out.push(Entry {
            name,
            data: &bytes[start..start + size],
            time: dos_time(date_raw, time_raw),
        });
    }
    Ok(out)
}

// ------------------------------------------------------------------ USDA

/// Every `(a, b, c)` triple of a field like `point3f[] points = [ … ]`. The field name already
/// contains `[]`, so the first `[` after it is the real list.
fn tripel(text: &str, field: &str) -> Option<Vec<[f64; 3]>> {
    let start = text.find(field)? + field.len();
    let rest = &text[start..];
    let onto = rest.find('[')?;
    let zu = rest[onto..].find(']')? + onto;
    let mut out = Vec::new();
    for group in rest[onto + 1..zu].split('(').skip(1) {
        let end = group.find(')')?;
        let numbers: Vec<f64> = group[..end]
            .split(',')
            .filter_map(|z| z.trim().parse::<f64>().ok())
            .collect();
        if numbers.len() == 3 {
            out.push([numbers[0], numbers[1], numbers[2]]);
        }
    }
    Some(out)
}

/// A list of integers from `int[] NAME = [ … ]`. The name must not include `[]`, or `find('[')`
/// hits the type's bracket and silently returns an empty list.
fn numbers(text: &str, field: &str) -> Option<Vec<usize>> {
    let start = text.find(field)? + field.len();
    let rest = &text[start..];
    let onto = rest.find('[')?;
    let zu = rest[onto..].find(']')? + onto;
    Some(
        rest[onto + 1..zu]
            .split(',')
            .filter_map(|z| z.trim().parse::<usize>().ok())
            .collect(),
    )
}

/// A `string NAME = "…"` from a USDA header.
fn string(text: &str, field: &str) -> Option<String> {
    let start = text.find(field)? + field.len();
    let rest = &text[start..];
    let onto = rest.find('"')? + 1;
    let zu = rest[onto..].find('"')? + onto;
    Some(rest[onto..zu].to_string())
}

/// The `matrix4d xformOp:transform`, without transposing, on purpose. The client reads the
/// translation from `transform[12..15]`; USD stores it in the fourth row of a row-major array, at
/// the same indices, because USD uses row vectors (`p' = p · M`) and the client column vectors
/// (`p' = M · p`). Transposing would move every element to (0, 0, 0) and look plausible;
/// `the_translation_lands_in_the_fourth_column` catches it.
fn transform(text: &str) -> Option<[f64; 16]> {
    let start = text.find("matrix4d xformOp:transform")?;
    let rest = &text[start..];
    let onto = rest.find('(')? + 1;
    // The matrix is four bracket groups; a fifth bracket closes the field.
    let mut lines = Vec::new();
    for line in rest[onto..].split('(').skip(1) {
        let end = line.find(')')?;
        let numbers: Vec<f64> = line[..end]
            .split(',')
            .filter_map(|z| z.trim().parse::<f64>().ok())
            .collect();
        if numbers.len() == 4 {
            lines.push(numbers);
        }
        if lines.len() == 4 {
            break;
        }
    }
    if lines.len() != 4 {
        return None;
    }
    let mut out = [0.0f64; 16];
    for (z, line) in lines.iter().enumerate() {
        for (s, wert) in line.iter().enumerate() {
            out[z * 4 + s] = *wert;
        }
    }
    Some(out)
}

/// One `…/Wall0.usda` as an element. The extent is the span of the points in the local frame,
/// the size before translation, which stays fixed if the capture is later reoriented.
pub fn parse_usda(name: &str, text: &str) -> Option<Element> {
    let points = tripel(text, "point3f[] points")?;
    if points.is_empty() {
        return None;
    }
    let transform = transform(text)?;
    let mut span = [0.0f64; 3];
    for axis in 0..3 {
        let min = points.iter().map(|p| p[axis]).fold(f64::INFINITY, f64::min);
        let max = points
            .iter()
            .map(|p| p[axis])
            .fold(f64::NEG_INFINITY, f64::max);
        span[axis] = max - min;
    }
    Some(Element {
        id: name
            .rsplit('/')
            .next()
            .unwrap_or(name)
            .trim_end_matches(".usda")
            .to_string(),
        category: string(text, "string Category").unwrap_or_else(|| "Unknown".to_string()),
        dimensions_m: span,
        transform,
        confidence: "low",
        source_id: string(text, "string UUID").unwrap_or_default(),
    })
}

/// Area of a flat body's upper side, from its triangles. Not `x * y`: the floor is an L-shape,
/// and its frame measures the hull. Summed over the triangles whose local normal points up (+z for
/// the flat floor plate); checked 2026-09-26 against the real capture, where a ring of edge points
/// gave 32.99 instead of 31.57 m².
pub fn top_area_m2(text: &str) -> Option<f64> {
    let points = tripel(text, "point3f[] points")?;
    let normals = tripel(text, "normal3f[] normals")?;
    let corners_per_surface = numbers(text, "faceVertexCounts")?;
    let indices = numbers(text, "faceVertexIndices")?;

    let mut total = 0.0;
    let mut o = 0usize;
    for n in corners_per_surface {
        if o + n > indices.len() {
            return None;
        }
        let corners = &indices[o..o + n];
        o += n;
        let up = corners
            .iter()
            .filter_map(|&i| normals.get(i))
            .map(|n| n[2])
            .sum::<f64>()
            / n as f64;
        if up <= 0.9 {
            continue;
        }
        for k in 1..n.saturating_sub(1) {
            let (a, b, c) = (
                points[corners[0]],
                points[corners[k]],
                points[corners[k + 1]],
            );
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let cross = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            total += (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt() / 2.0;
        }
    }
    Some(total)
}

/// Reads a USDZ from its path; the capture is large and stays where it is.
pub fn read_usdz(path: &Path) -> Result<Scan, ScanError> {
    let bytes = std::fs::read(path).map_err(|source| ScanError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let sha256 = {
        use sha2::{Digest, Sha256};
        Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let entries = zip_entries(&bytes, path)?;

    let mut elements = Vec::new();
    let mut mesh_assets = 0usize;
    let mut boden_usda = None;
    for e in &entries {
        if e.name.starts_with("assets/Mesh/") {
            mesh_assets += 1;
        }
        if !e.name.ends_with(".usda") || !e.name.starts_with("assets/Mesh/") {
            continue;
        }
        let text = std::str::from_utf8(e.data).map_err(|err| ScanError::Form {
            path: path.to_path_buf(),
            name: e.name.clone(),
            detail: format!("not UTF-8: {err}"),
        })?;
        let element = parse_usda(&e.name, text).ok_or_else(|| ScanError::Form {
            path: path.to_path_buf(),
            name: e.name.clone(),
            detail: "points, normals or transform missing".to_string(),
        })?;
        if element.category == "Floor" {
            boden_usda = Some(text.to_string());
        }
        elements.push(element);
    }

    let header = entries
        .iter()
        .find(|e| e.name == "CapturedRoom.usda")
        .ok_or_else(|| ScanError::Missing {
            path: path.to_path_buf(),
            name: "CapturedRoom.usda".to_string(),
        })?;
    let header_text = std::str::from_utf8(header.data).map_err(|err| ScanError::Form {
        path: path.to_path_buf(),
        name: header.name.clone(),
        detail: format!("not UTF-8: {err}"),
    })?;
    // Every room group has a `…_centerTop` beside it; counting those counts the groups.
    let room_groups = header_text.matches("_centerTop").count();

    elements.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Scan {
        elements,
        capture_finished_at: header.time.clone(),
        mesh_assets,
        room_groups,
        sha256,
        byte_length: bytes.len(),
        boden_usda,
    })
}

// ------------------------------------------------------------------ Captures on disk

/// `<assets_root>/captures/<flat>`, or an error naming the missing path.
pub fn captures_root(flat: &str) -> Result<PathBuf, ScanError> {
    let root = crate::model::assets_dir()
        .map_err(|e| ScanError::Assets(e.to_string()))?
        .join("captures")
        .join(flat);
    if root.is_dir() {
        Ok(root)
    } else {
        Err(ScanError::NoCapture(root.display().to_string()))
    }
}

/// Capture dates under a root, ascending; the name is the date, which sorts as a string.
pub fn capture_dates(flat: &str) -> Result<Vec<PathBuf>, ScanError> {
    let root = captures_root(flat)?;
    let mut dates: Vec<PathBuf> = std::fs::read_dir(&root)
        .map_err(|source| ScanError::Read {
            path: root.clone(),
            source,
        })?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dates.sort();
    Ok(dates)
}

/// The newest capture with both its USDZ and its observation, the same condition
/// `/api/roomplan/reference` uses: half a capture looks like a whole one.
pub fn latest_capture(flat: &str) -> Result<PathBuf, ScanError> {
    for dir in capture_dates(flat)?.into_iter().rev() {
        if dir.join("captured-room.usdz").is_file() && dir.join("observation.json").is_file() {
            return Ok(dir);
        }
    }
    Err(ScanError::NoCapture(format!(
        "no complete capture under {}",
        captures_root(flat)?.display()
    )))
}

/// Writes the draft as `draft.json` next to the capture, where `/api/roomplan/revisions` reads it.
/// An existing draft is replaced (it is derived from the USDZ); `review.json` and the USDZ itself
/// are not touched.
pub fn write_draft(
    scan: &Scan,
    flat: &str,
    capture_dir: &Path,
    today: &str,
) -> std::io::Result<PathBuf> {
    let file = capture_dir.join("draft.json");
    let content = draft(scan, flat, today);
    std::fs::write(&file, serde_json::to_vec_pretty(&content)?)?;
    Ok(file)
}

// ------------------------------------------------------------------ Draft

/// The draft as `roomplan-capture/v1`, as `/api/roomplan/revisions` expects and
/// `dashboard/src/lib/roomplan.ts` reads. `created_at` is the day of the draft;
/// `provenance.capture_finished_at` the day of the capture.
pub fn draft(scan: &Scan, flat: &str, created_at: &str) -> serde_json::Value {
    let als_json = |e: &Element| {
        serde_json::json!({
            "id": e.id,
            "category": e.category,
            "dimensions_m": e.dimensions_m,
            "transform": e.transform,
            "confidence": e.confidence,
            "source_id": e.source_id,
        })
    };
    let mut surfaces = Vec::new();
    let mut openings = Vec::new();
    let mut objects = Vec::new();
    for e in &scan.elements {
        if e.is_surface() {
            surfaces.push(als_json(e));
        } else if e.is_opening() {
            openings.push(als_json(e));
        } else {
            objects.push(als_json(e));
        }
    }
    let short = &scan.sha256[..16.min(scan.sha256.len())];
    serde_json::json!({
        "schema_version": "roomplan-capture/v1",
        "draft_id": format!("raw-{short}"),
        "created_at": created_at,
        "source": {
            "platform": "macos",
            "roomplan_version": "raw-usdz",
            "app_version": env!("CARGO_PKG_VERSION"),
        },
        "coordinate_system": { "units": "meters", "up_axis": "Y" },
        "room": {
            "id": flat,
            "surfaces": surfaces,
            "openings": openings,
            "objects": objects,
        },
        "assets": [{
            "asset_id": format!("usdz-{short}"),
            "role": "mesh",
            "format": "usdz",
            "byte_length": scan.byte_length,
            "sha256": scan.sha256,
            "storage_token": "captures",
        }],
        "provenance": {
            "capture_mode": "new_room",
            "capture_finished_at": scan.capture_finished_at,
            "notes": "Read from the raw USDZ. RoomPlan confidence is not carried by the USDZ, \
                      so every element is low. Room membership is not carried either.",
        },
    })
}

// ------------------------------------------------------------------ Comparison

/// A finding: what the scan says, what `room.toml` says, and how far apart they are. It claims
/// no identity between a scan element and a `room.toml` entry; matching them is a decision made in
/// review, not a computation.
#[derive(Debug, Clone, PartialEq)]
pub struct Diff {
    /// Wall height in cm when all walls agree.
    pub scan_wall_height_cm: Option<f64>,
    /// The height `room.toml` states; `0` there means not measured.
    pub model_height_cm: i32,
    /// Doors and windows from the scan: id, category, width and height in cm.
    pub scan_openings: Vec<(String, String, f64, f64)>,
    /// Doors and windows from `room.toml`: id, width in cm.
    pub model_openings: Vec<(String, i32)>,
    /// The floor the capture shows, as the area of its upper side.
    pub scan_floor_area_m2: Option<f64>,
    /// The inner floor area `room.toml` knows.
    pub model_inner_area_m2: f64,
    /// Total length of all scanned walls in cm. Not comparable to `room.toml`: the scan splits walls
    /// at their openings. Shown for reference only.
    pub scan_wall_length_cm: f64,
    /// Furniture and appliances from the scan: `Element (category)` and outer size in cm, in local
    /// frame order.
    pub scan_objects: Vec<(String, [f64; 3])>,
}

impl Diff {
    /// Where the scan has a number that `room.toml` lists as open.
    pub fn open_question_answered(&self) -> bool {
        self.scan_wall_height_cm.is_some() && self.model_height_cm == 0
    }
}

/// The inner floor area the model knows: the main room's polygon (the kitchen niche is part of
/// it) plus the bathroom, computed rather than read, as `Room::area_m2` is.
fn model_inner_area_m2(room: &Room) -> f64 {
    room.area_m2() + room.bathroom.as_ref().map_or(0.0, |bad| bad.area_m2)
}

/// The scan against the measured model. Computes, writes nothing.
pub fn diff(scan: &Scan, room: &Room) -> Diff {
    let mut openings: Vec<(String, String, f64, f64)> = scan
        .openings()
        .map(|e| {
            (
                e.id.clone(),
                e.category.clone(),
                e.dimensions_m[0] * 100.0,
                e.dimensions_m[1] * 100.0,
            )
        })
        .collect();
    openings.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    let mut objects: Vec<(String, [f64; 3])> = scan
        .objects()
        .map(|e| {
            (
                format!("{} ({})", e.id, e.category),
                [
                    e.dimensions_m[0] * 100.0,
                    e.dimensions_m[1] * 100.0,
                    e.dimensions_m[2] * 100.0,
                ],
            )
        })
        .collect();
    objects.sort_by(|a, b| a.0.cmp(&b.0));
    Diff {
        scan_wall_height_cm: scan.wall_height_m().map(|m| m * 100.0),
        model_height_cm: room.main_room.height,
        scan_openings: openings,
        model_openings: room
            .openings
            .iter()
            .map(|o| (o.id.clone(), o.width))
            .collect(),
        scan_floor_area_m2: scan.floor_area_m2(),
        model_inner_area_m2: model_inner_area_m2(room),
        scan_wall_length_cm: scan
            .surfaces()
            .filter(|e| e.category == "Wall")
            .map(|e| e.dimensions_m[0] * 100.0)
            .sum(),
        scan_objects: objects,
    }
}

/// The scan against a loaded model.
pub fn compare(scan: &Scan, model: &Model) -> Diff {
    diff(scan, &model.room)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ZIP with stored entries, as `zip -0` writes it; only as much format as the reader claims.
    fn zip(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        // A fixed timestamp, 1980-01-01 00:00, so the test does not depend on the clock.
        for (name, content) in entries {
            let offset = out.len() as u32;
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes()); // version
            out.extend_from_slice(&0u16.to_le_bytes()); // flags
            out.extend_from_slice(&METHOD_STORED.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // time
            out.extend_from_slice(&0x0021u16.to_le_bytes()); // date: 1980-01-01
            out.extend_from_slice(&0u32.to_le_bytes()); // CRC, not checked here
            out.extend_from_slice(&(content.len() as u32).to_le_bytes());
            out.extend_from_slice(&(content.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // extra
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(content.as_bytes());

            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes()); // version made by
            central.extend_from_slice(&20u16.to_le_bytes()); // version needed
            central.extend_from_slice(&0u16.to_le_bytes()); // flags
            central.extend_from_slice(&METHOD_STORED.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0x0021u16.to_le_bytes());
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&(content.len() as u32).to_le_bytes());
            central.extend_from_slice(&(content.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes()); // extra
            central.extend_from_slice(&0u16.to_le_bytes()); // comment
            central.extend_from_slice(&0u16.to_le_bytes()); // disk
            central.extend_from_slice(&0u16.to_le_bytes()); // attributes
            central.extend_from_slice(&0u32.to_le_bytes()); // external
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let central_start = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&central_start.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    /// A wall 3.00 m long, 2.50 m high and 0.16 m thick, translated by (1, 2, 3). Invented numbers.
    fn wand_usda() -> String {
        r#"#usda 1.0
(
    defaultPrim = "Wall0"
    metersPerUnit = 1
)
def Xform "Wall0" (
    customData = {
        string Category = "Wall"
        string UUID = "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE"
    }
)
{
    def Mesh "Wall0"
    {
        point3f[] points = [(0, 0, 0), (300, 0, 0), (300, 250, 0), (0, 250, 0), (0, 0, 16), (300, 0, 16), (300, 250, 16), (0, 250, 16)]
        matrix4d xformOp:transform = ( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (1, 2, 3, 1) )
        uniform token[] xformOpOrder = ["xformOp:transform"]
    }
}
"#
        .to_string()
    }

    #[test]
    fn a_stored_entry_is_read_and_its_timestamp_recognised() {
        let bytes = zip(&[("CapturedRoom.usda", "def Xform \"Section_grp\"\n")]);
        let path = std::env::temp_dir().join(format!("interior-zip-{}.usdz", std::process::id()));
        std::fs::write(&path, &bytes).expect("Testdatei");
        let entries = zip_entries(&bytes, &path).expect("read");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "CapturedRoom.usda");
        assert_eq!(
            entries[0].time.as_deref(),
            Some("1980-01-01T00:00:00"),
            "date 0x0021 is 1980-01-01"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// A compressed entry is an error, not an empty answer.
    #[test]
    fn a_compressed_entry_is_an_error() {
        let mut bytes = zip(&[("CapturedRoom.usda", "empty")]);
        // Set the method to 8 (deflate) in the local and the central header.
        let local = bytes.iter().position(|_| true).unwrap();
        bytes[local + 8] = 8;
        let central_start = u32::from_le_bytes([
            bytes[bytes.len() - 6],
            bytes[bytes.len() - 5],
            bytes[bytes.len() - 4],
            bytes[bytes.len() - 3],
        ]) as usize;
        bytes[central_start + 10] = 8;
        let path =
            std::env::temp_dir().join(format!("interior-deflate-{}.usdz", std::process::id()));
        std::fs::write(&path, &bytes).expect("Testdatei");
        match zip_entries(&bytes, &path) {
            Err(ScanError::Compressed { method, .. }) => assert_eq!(method, 8),
            Err(other) => panic!("expected Compressed, got: {other:?}"),
            // No `{:?}` on success: it carries the payload.
            Ok(read_entries) => panic!("expected an error, read {} entries", read_entries.len()),
        }
        let _ = std::fs::remove_file(&path);
    }

    /// The translation must land in `[12..15]`; read wrongly, nothing fails.
    #[test]
    fn the_translation_lands_in_the_fourth_column() {
        let e = parse_usda("assets/Mesh/Walls/Wall0/Wall0.usda", &wand_usda()).expect("Element");
        assert_eq!(e.id, "Wall0");
        assert_eq!(e.category, "Wall");
        assert_eq!(
            e.source_id, "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE",
            "the native UUID is the identity, not the file name"
        );
        assert_eq!(e.position_m(), [1.0, 2.0, 3.0]);
        assert_eq!(e.confidence, "low", "the USDZ carries no confidence");
    }

    #[test]
    fn the_extent_comes_from_the_local_frame_in_metres() {
        let e = parse_usda("assets/Mesh/Walls/Wall0/Wall0.usda", &wand_usda()).expect("Element");
        // Points written in centimetres and meant as metres by metersPerUnit = 1: the span is the
        // difference of the points, unchanged.
        assert_eq!(e.dimensions_m, [300.0, 250.0, 16.0]);
    }

    #[test]
    fn categories_are_separated() {
        let window = r#"def Xform "Window0" ( customData = { string Category = "Window" } )
{ def Mesh "Window0" {
    point3f[] points = [(0, 0, 0), (90, 0, 0), (90, 120, 0)]
    matrix4d xformOp:transform = ( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1) )
} }"#;
        let w = parse_usda("Wall0.usda", &wand_usda()).expect("Wand");
        let f = parse_usda("Window0.usda", window).expect("Fenster");
        assert!(w.is_surface() && !w.is_opening() && !w.is_object());
        assert!(f.is_opening() && !f.is_door());
        assert!(f.dimensions_m[0] > 0.0);
    }

    #[test]
    fn a_closed_door_is_still_a_door() {
        let door = r#"def Xform "Door0" ( customData = { string Category = "Door(Isopen: False)" } )
{ def Mesh "Door0" {
    point3f[] points = [(0, 0, 0), (90, 0, 0), (90, 200, 0)]
    matrix4d xformOp:transform = ( (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 1, 0), (0, 0, 0, 1) )
} }"#;
        let t = parse_usda("Door0.usda", door).expect("door");
        assert!(
            t.is_door(),
            "the state inside the category does not make it something else"
        );
    }

    /// Flat bodies: their area is not their span. Two equal plates sum to their total, while the
    /// span of the same body would be twice that.
    #[test]
    fn the_floor_area_comes_from_the_triangles_not_the_span() {
        // Two 2 x 3 plates side by side: total span 4 x 3, area 12.
        let text = r#"def Xform "Floor0" {
    def Mesh "Floor0"
    {
        int[] faceVertexCounts = [3, 3, 3, 3]
        int[] faceVertexIndices = [0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]
        normal3f[] normals = [(0, 0, 1), (0, 0, 1), (0, 0, 1), (0, 0, 1), (0, 0, 1), (0, 0, 1), (0, 0, 1), (0, 0, 1)]
        point3f[] points = [(0, 0, 0), (2, 0, 0), (2, 3, 0), (0, 3, 0), (2, 0, 0), (4, 0, 0), (4, 3, 0), (2, 3, 0)]
    }
}"#;
        let area = top_area_m2(text).expect("area");
        assert!((area - 12.0).abs() < 1e-9, "expected 12, got {area}");
    }

    /// Only upward triangles count, or the area doubles. The second triangle uses points 3 to 5:
    /// mesh output that shares two faces of different direction duplicates the points, as the real
    /// floor does (28 facets on 84 points).
    #[test]
    fn the_underside_does_not_count() {
        let text = r#"def Mesh "Floor0"
{
    int[] faceVertexCounts = [3, 3]
    int[] faceVertexIndices = [0, 1, 2, 3, 4, 5]
    normal3f[] normals = [(0, 0, 1), (0, 0, 1), (0, 0, 1), (0, 0, -1), (0, 0, -1), (0, 0, -1)]
    point3f[] points = [(0, 0, 0), (2, 0, 0), (0, 3, 0), (0, 0, 0), (2, 0, 0), (0, 3, 0)]
}"#;
        let area = top_area_m2(text).expect("area");
        assert!((area - 3.0).abs() < 1e-9, "expected 3, got {area}");
    }

    /// A contradicting scan gives no number.
    #[test]
    fn contradicting_walls_give_no_height() {
        let mut a = parse_usda("Wall0.usda", &wand_usda()).expect("Wand");
        let mut b = a.clone();
        b.dimensions_m[1] = 111.0;
        a.dimensions_m[1] = 222.0;
        let scan = Scan {
            elements: vec![a, b],
            ..Default::default()
        };
        assert_eq!(scan.wall_height_m(), None);
        let one = Scan {
            elements: vec![parse_usda("Wall0.usda", &wand_usda()).expect("Wand")],
            ..Default::default()
        };
        assert_eq!(one.wall_height_m(), Some(250.0));
    }

    /// The draft must have the shape `roomplan.ts` reads; a misspelt `schema_version` fails nowhere,
    /// the draft simply never arrives.
    #[test]
    fn the_draft_has_the_client_shape() {
        let scan = Scan {
            elements: vec![
                parse_usda("Wall0.usda", &wand_usda()).expect("Wand"),
                parse_usda("Bed0.usda", &wand_usda().replace("Wall", "Bed")).expect("Bett"),
            ],
            sha256: "a".repeat(64),
            byte_length: 123,
            ..Default::default()
        };
        let d = draft(&scan, "muster", "2026-09-26");
        assert_eq!(d["schema_version"], "roomplan-capture/v1");
        assert_eq!(d["coordinate_system"]["units"], "meters");
        assert_eq!(d["room"]["id"], "muster");
        assert_eq!(d["room"]["surfaces"].as_array().map(|a| a.len()), Some(1));
        assert_eq!(d["room"]["openings"].as_array().map(|a| a.len()), Some(0));
        assert_eq!(d["room"]["objects"].as_array().map(|a| a.len()), Some(1));
        let e = &d["room"]["surfaces"][0];
        assert_eq!(
            e["transform"][12], 1.0,
            "the client reads the translation here"
        );
        assert_eq!(e["confidence"], "low");
    }
}
