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

#[derive(Clone, Copy)]
struct WallAnchor {
    center: [f64; 2],
    length: f64,
    angle: f64,
}

#[derive(Clone, Copy)]
struct Alignment {
    angle: f64,
    translation: [f64; 2],
    wall_matches: usize,
    rms_error: f64,
    ambiguous: bool,
}

fn wall_anchors(scan: &Scan) -> Vec<WallAnchor> {
    scan.surfaces()
        .filter(|element| element.category == "Wall")
        .filter_map(|element| {
            let transform = &element.transform;
            let axis_length = transform[0].hypot(transform[2]);
            if !axis_length.is_finite() || axis_length < 1e-6 {
                return None;
            }
            Some(WallAnchor {
                center: [transform[12], transform[14]],
                length: element.dimensions_m[0],
                angle: transform[2].atan2(transform[0]),
            })
        })
        .collect()
}

#[derive(Clone)]
struct ModelOpeningAnchor {
    category: String,
    center: [f64; 2],
    width: f64,
    angle: f64,
}

#[derive(Clone, Copy)]
struct FlatAlignment {
    angle: f64,
    translation: [f64; 2],
    matching_walls: usize,
    matching_openings: usize,
    error_m: f64,
    ambiguous: bool,
}

fn room_wall_anchors(room: &Room) -> Vec<WallAnchor> {
    room.walls
        .values()
        .filter_map(|wall| {
            let dx = (wall.to[0] - wall.from[0]) as f64 / 100.0;
            let dy = (wall.to[1] - wall.from[1]) as f64 / 100.0;
            let length = dx.hypot(dy);
            (length > 0.05).then_some(WallAnchor {
                center: [
                    (wall.from[0] + wall.to[0]) as f64 / 200.0,
                    (wall.from[1] + wall.to[1]) as f64 / 200.0,
                ],
                length,
                angle: dy.atan2(dx),
            })
        })
        .collect()
}

fn scan_wall_runs(scan: &Scan) -> Vec<WallAnchor> {
    let mut remaining = wall_anchors(scan);
    let mut runs = Vec::new();
    while let Some(first) = remaining.pop() {
        let angle = first.angle;
        let (sin, cos) = angle.sin_cos();
        let direction = [cos, sin];
        let normal = [-sin, cos];
        let mut group = vec![first];
        loop {
            let (group_start, group_end) =
                group
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(start, end), part| {
                        let along = part.center[0] * direction[0] + part.center[1] * direction[1];
                        (
                            start.min(along - part.length / 2.0),
                            end.max(along + part.length / 2.0),
                        )
                    });
            let next = remaining.iter().position(|part| {
                if wall_axis_distance(angle, part.angle) > 0.08 {
                    return false;
                }
                let across = (part.center[0] - first.center[0]) * normal[0]
                    + (part.center[1] - first.center[1]) * normal[1];
                if across.abs() > 0.14 {
                    return false;
                }
                let along = part.center[0] * direction[0] + part.center[1] * direction[1];
                let start = along - part.length / 2.0;
                let end = along + part.length / 2.0;
                start <= group_end + 1.4 && end >= group_start - 1.4
            });
            let Some(index) = next else {
                break;
            };
            group.push(remaining.swap_remove(index));
        }
        let (start, end, offset, weight) = group.iter().fold(
            (f64::INFINITY, f64::NEG_INFINITY, 0.0, 0.0),
            |(start, end, offset, weight), part| {
                let along = part.center[0] * direction[0] + part.center[1] * direction[1];
                let part_weight = part.length.max(0.01);
                (
                    start.min(along - part.length / 2.0),
                    end.max(along + part.length / 2.0),
                    offset
                        + (part.center[0] * normal[0] + part.center[1] * normal[1]) * part_weight,
                    weight + part_weight,
                )
            },
        );
        let along = (start + end) / 2.0;
        let across = offset / weight.max(0.01);
        runs.push(WallAnchor {
            center: [
                direction[0] * along + normal[0] * across,
                direction[1] * along + normal[1] * across,
            ],
            length: end - start,
            angle,
        });
    }
    runs
}

fn room_opening_anchors(room: &Room) -> Vec<ModelOpeningAnchor> {
    room.openings
        .iter()
        .filter_map(|opening| {
            let wall = room.walls.get(&opening.wall)?;
            let (from, to) = room.opening_span(opening)?;
            Some(ModelOpeningAnchor {
                category: opening.kind.clone().unwrap_or_default(),
                center: [
                    (from[0] + to[0]) as f64 / 200.0,
                    (from[1] + to[1]) as f64 / 200.0,
                ],
                width: opening.width as f64 / 100.0,
                angle: ((wall.to[1] - wall.from[1]) as f64)
                    .atan2((wall.to[0] - wall.from[0]) as f64),
            })
        })
        .collect()
}

fn opening_families_match(scan_category: &str, model_category: &str) -> bool {
    if scan_category.starts_with("Window") {
        model_category.starts_with("window")
    } else if scan_category.starts_with("Door") || scan_category.starts_with("Opening") {
        model_category.starts_with("door") || model_category.starts_with("glass_door")
    } else {
        false
    }
}

fn transform_point(alignment: FlatAlignment, point: [f64; 2]) -> [f64; 2] {
    let (sin, cos) = alignment.angle.sin_cos();
    [
        cos * point[0] - sin * point[1] + alignment.translation[0],
        sin * point[0] + cos * point[1] + alignment.translation[1],
    ]
}

fn score_flat_alignment(
    candidate: FlatAlignment,
    scan_walls: &[WallAnchor],
    model_walls: &[WallAnchor],
    scan_openings: &[&Element],
    model_openings: &[ModelOpeningAnchor],
) -> FlatAlignment {
    let mut used_walls = Vec::new();
    let mut residuals = Vec::new();
    for scan_wall in scan_walls {
        let center = transform_point(candidate, scan_wall.center);
        let angle = candidate.angle + scan_wall.angle;
        let best = model_walls
            .iter()
            .enumerate()
            .filter(|(index, _)| !used_walls.contains(index))
            .filter_map(|(index, model_wall)| {
                let model = *model_wall;
                let angle_error = wall_axis_distance(angle, model.angle);
                let length_error = (scan_wall.length - model.length).abs();
                let center_error = (center[0] - model.center[0]).hypot(center[1] - model.center[1]);
                (angle_error <= 0.18 && length_error <= 0.35 && center_error <= 0.35).then_some((
                    index,
                    center_error.hypot(length_error),
                    center_error.hypot(length_error),
                ))
            })
            .min_by(|left, right| left.1.total_cmp(&right.1));
        if let Some((index, _, error)) = best {
            used_walls.push(index);
            residuals.push(error);
        }
    }

    let mut used_openings = Vec::new();
    for opening in scan_openings {
        let position = opening.position_m();
        let center = transform_point(candidate, [position[0], position[2]]);
        let angle = candidate.angle + opening.transform[2].atan2(opening.transform[0]);
        let best = model_openings
            .iter()
            .enumerate()
            .filter(|(index, _)| !used_openings.contains(index))
            .filter_map(|(index, model)| {
                if !opening_families_match(&opening.category, &model.category) {
                    return None;
                }
                let center_error = (center[0] - model.center[0]).hypot(center[1] - model.center[1]);
                let width_error = (opening.dimensions_m[0] - model.width).abs();
                let angle_error = wall_axis_distance(angle, model.angle);
                (center_error <= 0.35 && width_error <= 0.30 && angle_error <= 0.25).then_some((
                    index,
                    center_error.hypot(width_error),
                    center_error.hypot(width_error),
                ))
            })
            .min_by(|left, right| left.1.total_cmp(&right.1));
        if let Some((index, _, error)) = best {
            used_openings.push(index);
            residuals.push(error);
        }
    }
    let error_m = if residuals.is_empty() {
        f64::INFINITY
    } else {
        (residuals.iter().map(|error| error.powi(2)).sum::<f64>() / residuals.len() as f64).sqrt()
    };
    FlatAlignment {
        matching_walls: used_walls.len(),
        matching_openings: used_openings.len(),
        error_m,
        ..candidate
    }
}

fn align_scan_to_flat(scan: &Scan, room: &Room) -> Option<FlatAlignment> {
    let scan_walls = scan_wall_runs(scan);
    let model_walls = room_wall_anchors(room);
    let scan_openings = scan.openings().collect::<Vec<_>>();
    let model_openings = room_opening_anchors(room);
    let mut hypotheses = Vec::new();

    for scan_wall in &scan_walls {
        for model_wall in &model_walls {
            if (scan_wall.length - model_wall.length).abs() > 0.35 {
                continue;
            }
            for direction in [-1.0, 1.0] {
                let angle = model_wall.angle - scan_wall.angle
                    + if direction < 0.0 {
                        std::f64::consts::PI
                    } else {
                        0.0
                    };
                let (sin, cos) = angle.sin_cos();
                let rotated = [
                    cos * scan_wall.center[0] - sin * scan_wall.center[1],
                    sin * scan_wall.center[0] + cos * scan_wall.center[1],
                ];
                hypotheses.push(FlatAlignment {
                    angle,
                    translation: [
                        model_wall.center[0] - rotated[0],
                        model_wall.center[1] - rotated[1],
                    ],
                    matching_walls: 0,
                    matching_openings: 0,
                    error_m: f64::INFINITY,
                    ambiguous: false,
                });
            }
        }
    }
    for scan_opening in &scan_openings {
        for model_opening in &model_openings {
            if !opening_families_match(&scan_opening.category, &model_opening.category)
                || (scan_opening.dimensions_m[0] - model_opening.width).abs() > 0.30
            {
                continue;
            }
            for direction in [-1.0, 1.0] {
                let angle = model_opening.angle
                    - scan_opening.transform[2].atan2(scan_opening.transform[0])
                    + if direction < 0.0 {
                        std::f64::consts::PI
                    } else {
                        0.0
                    };
                let (sin, cos) = angle.sin_cos();
                let position = scan_opening.position_m();
                let rotated = [
                    cos * position[0] - sin * position[2],
                    sin * position[0] + cos * position[2],
                ];
                hypotheses.push(FlatAlignment {
                    angle,
                    translation: [
                        model_opening.center[0] - rotated[0],
                        model_opening.center[1] - rotated[1],
                    ],
                    matching_walls: 0,
                    matching_openings: 0,
                    error_m: f64::INFINITY,
                    ambiguous: false,
                });
            }
        }
    }

    let mut candidates = hypotheses
        .into_iter()
        .map(|candidate| {
            score_flat_alignment(
                candidate,
                &scan_walls,
                &model_walls,
                &scan_openings,
                &model_openings,
            )
        })
        .filter(|candidate| {
            candidate.matching_walls >= 2
                || (candidate.matching_walls >= 1 && candidate.matching_openings >= 1)
                || candidate.matching_openings >= 2
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .matching_walls
            .cmp(&left.matching_walls)
            .then_with(|| right.matching_openings.cmp(&left.matching_openings))
            .then_with(|| left.error_m.total_cmp(&right.error_m))
    });
    let best = *candidates.first()?;
    let ambiguous = candidates.iter().skip(1).any(|candidate| {
        candidate.matching_walls == best.matching_walls
            && candidate.matching_openings == best.matching_openings
            && candidate.error_m <= best.error_m + 0.04
            && (wall_axis_distance(candidate.angle, best.angle) > 0.12
                || (candidate.translation[0] - best.translation[0])
                    .hypot(candidate.translation[1] - best.translation[1])
                    > 0.20)
    });
    Some(FlatAlignment { ambiguous, ..best })
}

/// Estimates capture coordinates in the measured flat frame; uncertain fits withhold the transform.
pub fn flat_alignment(scan: &Scan, room: &Room) -> serde_json::Value {
    let Some(alignment) = align_scan_to_flat(scan, room) else {
        return serde_json::json!({
            "status": "uncertain",
            "matching_walls": 0,
            "matching_openings": 0,
            "error_m": null,
            "ambiguous": false,
            "reason": "The scan does not contain enough matching wall or opening anchors."
        });
    };
    let status = if alignment.error_m <= 0.15 && !alignment.ambiguous {
        "aligned"
    } else {
        "uncertain"
    };
    let mut report = serde_json::json!({
        "status": status,
        "matching_walls": alignment.matching_walls,
        "matching_openings": alignment.matching_openings,
        "error_m": alignment.error_m,
        "ambiguous": alignment.ambiguous,
        "coordinates": "room.toml centimetres; origin at inner north-west; x east, y south",
        "reason": if status == "aligned" { "" } else if alignment.ambiguous { "Several scan-to-flat transforms fit equally well; placement coordinates were withheld." } else { "Scan-to-flat anchor error is too high; placement coordinates were withheld." }
    });
    if status == "aligned" {
        report["rotation_rad"] = serde_json::json!(alignment.angle);
        report["translation_m"] = serde_json::json!(alignment.translation);
    }
    report
}

/// Maps a scanned object's centre and orientation into a reviewed flat-plan placement.
pub fn flat_position_proposal(
    scan: &Scan,
    room: &Room,
    observation_id: &str,
    width_cm: i32,
    depth_cm: i32,
) -> Option<serde_json::Value> {
    if width_cm <= 0 || depth_cm <= 0 {
        return None;
    }
    let alignment = align_scan_to_flat(scan, room)?;
    if alignment.error_m > 0.15 || alignment.ambiguous {
        return None;
    }
    let object = scan.objects().find(|object| object.id == observation_id)?;
    let position = object.position_m();
    let center = transform_point(alignment, [position[0], position[2]]);
    let angle = alignment.angle + object.transform[2].atan2(object.transform[0]);
    let rot = ((angle.to_degrees() / 90.0).round() as i32 * 90).rem_euclid(360);
    let (placed_width, placed_depth) = if rot.rem_euclid(180) == 90 {
        (depth_cm, width_cm)
    } else {
        (width_cm, depth_cm)
    };
    Some(serde_json::json!({
        "x": (center[0] * 100.0 - placed_width as f64 / 2.0).round() as i32,
        "y": (center[1] * 100.0 - placed_depth as f64 / 2.0).round() as i32,
        "rot": rot,
        "center_x_cm": (center[0] * 100.0).round() as i32,
        "center_y_cm": (center[1] * 100.0).round() as i32,
        "observed_width_cm": (object.dimensions_m[0] * 100.0).round() as i32,
        "observed_depth_cm": (object.dimensions_m[2] * 100.0).round() as i32,
        "alignment_error_m": alignment.error_m
    }))
}

fn angular_distance(left: f64, right: f64) -> f64 {
    let difference = (left - right).rem_euclid(std::f64::consts::TAU);
    difference.min(std::f64::consts::TAU - difference)
}

fn wall_axis_distance(left: f64, right: f64) -> f64 {
    let difference = angular_distance(left, right);
    difference.min((std::f64::consts::PI - difference).abs())
}

fn align_scans(before: &Scan, after: &Scan) -> Option<Alignment> {
    let before_walls = wall_anchors(before);
    let after_walls = wall_anchors(after);
    let mut hypotheses = Vec::new();
    for before_wall in &before_walls {
        for after_wall in &after_walls {
            if (before_wall.length - after_wall.length).abs() > 0.18 {
                continue;
            }
            for direction in [-1.0, 1.0] {
                let angle = before_wall.angle - after_wall.angle
                    + if direction < 0.0 {
                        std::f64::consts::PI
                    } else {
                        0.0
                    };
                let (sin, cos) = angle.sin_cos();
                let rotated = [
                    cos * after_wall.center[0] - sin * after_wall.center[1],
                    sin * after_wall.center[0] + cos * after_wall.center[1],
                ];
                hypotheses.push(Alignment {
                    angle,
                    translation: [
                        before_wall.center[0] - rotated[0],
                        before_wall.center[1] - rotated[1],
                    ],
                    wall_matches: 0,
                    rms_error: f64::INFINITY,
                    ambiguous: false,
                });
            }
        }
    }

    for candidate in &mut hypotheses {
        let (sin, cos) = candidate.angle.sin_cos();
        let mut used_before = Vec::new();
        let mut residuals = Vec::new();
        for after_wall in &after_walls {
            let rotated = [
                cos * after_wall.center[0] - sin * after_wall.center[1] + candidate.translation[0],
                sin * after_wall.center[0] + cos * after_wall.center[1] + candidate.translation[1],
            ];
            let best = before_walls
                .iter()
                .enumerate()
                .filter(|(index, _)| !used_before.contains(index))
                .filter_map(|(index, before_wall)| {
                    let distance = (before_wall.center[0] - rotated[0])
                        .hypot(before_wall.center[1] - rotated[1]);
                    let length_delta = (before_wall.length - after_wall.length).abs();
                    let axis_delta =
                        wall_axis_distance(before_wall.angle, after_wall.angle + candidate.angle);
                    (distance <= 0.22 && length_delta <= 0.18 && axis_delta <= 0.20)
                        .then_some((index, distance))
                })
                .min_by(|left, right| left.1.total_cmp(&right.1));
            if let Some((index, distance)) = best {
                used_before.push(index);
                residuals.push(distance);
            }
        }
        candidate.wall_matches = residuals.len();
        candidate.rms_error = if residuals.is_empty() {
            f64::INFINITY
        } else {
            (residuals.iter().map(|error| error * error).sum::<f64>() / residuals.len() as f64)
                .sqrt()
        };
    }
    hypotheses.retain(|candidate| candidate.wall_matches >= 2);
    hypotheses.sort_by(|left, right| {
        right
            .wall_matches
            .cmp(&left.wall_matches)
            .then_with(|| left.rms_error.total_cmp(&right.rms_error))
    });
    let best = *hypotheses.first()?;
    let ambiguous = hypotheses.iter().skip(1).any(|candidate| {
        candidate.wall_matches == best.wall_matches
            && candidate.rms_error <= best.rms_error + 0.03
            && (angular_distance(candidate.angle, best.angle) > 0.087
                || (candidate.translation[0] - best.translation[0])
                    .hypot(candidate.translation[1] - best.translation[1])
                    > 0.10)
    });
    Some(Alignment { ambiguous, ..best })
}

/// Compare two captures without assigning furniture identity or writing placement state.
/// Wall geometry supplies a best-effort 2D rigid alignment; every furniture pairing remains a
/// suggestion that a person must confirm because RoomPlan IDs are capture-local.
pub fn reconcile(before: &Scan, after: &Scan) -> serde_json::Value {
    let Some(alignment) = align_scans(before, after) else {
        return serde_json::json!({
            "alignment": {
                "status": "uncertain",
                "matching_walls": 0,
                "error_m": null,
                "reason": "At least two matching wall anchors are needed to align these scans."
            },
            "matches": [],
            "unmatched_before": before.objects().map(|item| item.id.clone()).collect::<Vec<_>>(),
            "unmatched_after": after.objects().map(|item| item.id.clone()).collect::<Vec<_>>(),
            "human_confirmation_required": true
        });
    };

    let before_wall_count = before_walls_count(before);
    let after_wall_count = after_walls_count(after);
    let status = if alignment.rms_error <= 0.15 && !alignment.ambiguous {
        "aligned"
    } else {
        "uncertain"
    };
    let (sin, cos) = alignment.angle.sin_cos();
    let before_objects = before.objects().collect::<Vec<_>>();
    let after_objects = after.objects().collect::<Vec<_>>();
    let mut matches = Vec::new();
    let mut used_before = Vec::new();
    if status == "aligned" {
        for after_item in &after_objects {
            let position = after_item.position_m();
            let aligned = [
                cos * position[0] - sin * position[2] + alignment.translation[0],
                sin * position[0] + cos * position[2] + alignment.translation[1],
            ];
            let mut candidates = before_objects
                .iter()
                .enumerate()
                .filter(|(index, before_item)| {
                    !used_before.contains(index)
                        && category_family(&before_item.category)
                            == category_family(&after_item.category)
                        && dimensions_close(before_item, after_item)
                })
                .filter_map(|(index, before_item)| {
                    let old = before_item.position_m();
                    let distance = (old[0] - aligned[0]).hypot(old[2] - aligned[1]);
                    (distance <= 3.0).then_some((index, before_item, distance))
                })
                .collect::<Vec<_>>();
            candidates.sort_by(|left, right| left.2.total_cmp(&right.2));
            if let Some((index, before_item, distance)) = candidates.first().copied() {
                used_before.push(index);
                matches.push(serde_json::json!({
                    "before_observation_id": before_item.id,
                    "after_observation_id": after_item.id,
                    "category": after_item.category,
                    "distance_m": distance,
                    "suggestion": if distance <= 0.15 { "possible_same_position" } else { "possible_movement" },
                    "identity": "unconfirmed_candidate",
                    "human_confirmation_required": true,
                    "competing_candidates": candidates.iter().skip(1).filter(|candidate| candidate.2 - distance < 0.15).count()
                }));
            }
        }
    }
    let unmatched_before = before_objects
        .iter()
        .enumerate()
        .filter(|(index, _)| !used_before.contains(index))
        .map(|(_, item)| item.id.clone())
        .collect::<Vec<_>>();
    let matched_after = matches
        .iter()
        .filter_map(|entry| entry["after_observation_id"].as_str())
        .collect::<Vec<_>>();
    let unmatched_after = after_objects
        .iter()
        .filter(|item| !matched_after.contains(&item.id.as_str()))
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    serde_json::json!({
        "alignment": {
            "status": status,
            "matching_walls": alignment.wall_matches,
            "before_wall_anchors": before_wall_count,
            "after_wall_anchors": after_wall_count,
            "error_m": alignment.rms_error,
            "rotation_rad": alignment.angle,
            "translation_m": alignment.translation,
            "ambiguous": alignment.ambiguous,
            "reason": if status == "aligned" { "" } else if alignment.ambiguous { "Several wall alignments fit equally well; object suggestions were withheld." } else { "Wall alignment error is too high; object suggestions were withheld." }
        },
        "matches": matches,
        "unmatched_before": unmatched_before,
        "unmatched_after": unmatched_after,
        "unmatched_are_observations_not_confirmed_additions_or_removals": true,
        "human_confirmation_required": true
    })
}

fn before_walls_count(scan: &Scan) -> usize {
    wall_anchors(scan).len()
}

fn after_walls_count(scan: &Scan) -> usize {
    wall_anchors(scan).len()
}

fn category_family(category: &str) -> String {
    category
        .split('(')
        .next()
        .unwrap_or(category)
        .trim()
        .to_ascii_lowercase()
}

fn dimensions_close(left: &Element, right: &Element) -> bool {
    let mut left_horizontal = [left.dimensions_m[0], left.dimensions_m[2]];
    let mut right_horizontal = [right.dimensions_m[0], right.dimensions_m[2]];
    left_horizontal.sort_by(f64::total_cmp);
    right_horizontal.sort_by(f64::total_cmp);
    (left.dimensions_m[1] - right.dimensions_m[1]).abs() <= 0.20
        && (left_horizontal[0] - right_horizontal[0]).abs() <= 0.20
        && (left_horizontal[1] - right_horizontal[1]).abs() <= 0.20
}

/// The comparison as one document, for the CLI (`interior roomplan --json`), for the endpoint that
/// takes a capture from the phone, and for anything after them.
///
/// One shape and three callers on purpose: a copy of this literal in `main.rs` and another in
/// `api.rs` would be two answers to "how far apart are the scan and the flat", and only one of them
/// could be right. `capture` is the caller's name for the capture — a path for the CLI, the draft id
/// for the phone, which never sees an absolute path on this Mac.
pub fn comparison_json(scan: &Scan, d: &Diff, capture: &str) -> serde_json::Value {
    serde_json::json!({
        "capture": capture,
        "sha256": scan.sha256,
        "byte_length": scan.byte_length,
        "mesh_assets": scan.mesh_assets,
        "room_groups": scan.room_groups,
        "capture_finished_at": scan.capture_finished_at,
        "categories": scan.categories(),
        "scan_wall_height_cm": d.scan_wall_height_cm,
        "scan_wall_thickness_cm": scan.wall_thickness_m().map(|m| m * 100.0),
        "model_height_cm": d.model_height_cm,
        "scan_wall_length_cm": d.scan_wall_length_cm,
        "scan_openings": d.scan_openings.iter().map(|(id, cat, b, h)| serde_json::json!({
            "id": id, "category": cat, "width_cm": b, "height_cm": h
        })).collect::<Vec<_>>(),
        "model_openings": d.model_openings.iter().map(|(id, b)| serde_json::json!({
            "id": id, "width_cm": b
        })).collect::<Vec<_>>(),
        "scan_floor_area_m2": d.scan_floor_area_m2,
        "model_inner_area_m2": d.model_inner_area_m2,
        "scan_objects": d.scan_objects.iter().map(|(n, m)| serde_json::json!({
            "element": n, "cm": m
        })).collect::<Vec<_>>(),
    })
}

/// Fixtures the reader's own tests and the ingest endpoint's share.
///
/// The endpoint is tested with a file the reader accepts, not with a stub that only looks like one:
/// a ZIP as `zip -0` writes it, and one invented wall.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::METHOD_STORED;

    /// A ZIP with stored entries; only as much format as the reader claims.
    pub(crate) fn stored_zip(entries: &[(&str, &str)]) -> Vec<u8> {
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
    pub(crate) fn wall_usda() -> String {
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
}

#[cfg(test)]
mod tests {
    use super::fixtures::{stored_zip, wall_usda};
    use super::*;

    #[test]
    fn a_stored_entry_is_read_and_its_timestamp_recognised() {
        let bytes = stored_zip(&[("CapturedRoom.usda", "def Xform \"Section_grp\"\n")]);
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
        let mut bytes = stored_zip(&[("CapturedRoom.usda", "empty")]);
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
        let e = parse_usda("assets/Mesh/Walls/Wall0/Wall0.usda", &wall_usda()).expect("Element");
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
        let e = parse_usda("assets/Mesh/Walls/Wall0/Wall0.usda", &wall_usda()).expect("Element");
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
        let w = parse_usda("Wall0.usda", &wall_usda()).expect("Wand");
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
        let mut a = parse_usda("Wall0.usda", &wall_usda()).expect("Wand");
        let mut b = a.clone();
        b.dimensions_m[1] = 111.0;
        a.dimensions_m[1] = 222.0;
        let scan = Scan {
            elements: vec![a, b],
            ..Default::default()
        };
        assert_eq!(scan.wall_height_m(), None);
        let one = Scan {
            elements: vec![parse_usda("Wall0.usda", &wall_usda()).expect("Wand")],
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
                parse_usda("Wall0.usda", &wall_usda()).expect("Wand"),
                parse_usda("Bed0.usda", &wall_usda().replace("Wall", "Bed")).expect("Bett"),
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

    /// The comparison document is one shape for three callers, and its `capture` field is the caller's
    /// own name for the capture: a path for the CLI, a draft id for the phone, which never sees this
    /// Mac's absolute paths.
    #[test]
    fn the_comparison_names_the_capture_as_its_caller_does() {
        let scan = Scan::default();
        let d = Diff {
            scan_wall_height_cm: Some(250.0),
            model_height_cm: 0,
            scan_openings: vec![("T1".to_string(), "Door".to_string(), 88.0, 205.0)],
            model_openings: Vec::new(),
            scan_floor_area_m2: None,
            model_inner_area_m2: 21.5,
            scan_wall_length_cm: 1200.0,
            scan_objects: Vec::new(),
        };
        let json = comparison_json(&scan, &d, "2026-10-24T120000");
        assert_eq!(json["capture"], "2026-10-24T120000");
        assert_eq!(json["scan_wall_height_cm"], 250.0);
        assert_eq!(
            json["model_height_cm"], 0,
            "0 in room.toml means not measured"
        );
        assert_eq!(json["scan_openings"][0]["category"], "Door");
        assert_eq!(json["scan_openings"][0]["width_cm"], 88.0);
        assert!(
            json["scan_wall_thickness_cm"].is_null(),
            "a scan with no walls has no thickness"
        );
        assert_eq!(
            json["categories"],
            serde_json::json!({}),
            "an empty scan is an empty catalogue"
        );
    }
}
