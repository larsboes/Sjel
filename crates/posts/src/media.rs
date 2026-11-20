//! Photos selection and export through `osascript`, and resizing and collages through
//! ImageMagick (`magick`). Every external call is a plain argv: no shell runs.

use crate::{photos_id, Accent, Collage, Layout, Res, Role, Slot};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Long edge of an exported photo, in pixels.
pub const LONG_EDGE: u32 = 2400;
const PAD: u32 = 60;
const GAP: u32 = 36;
const RADIUS: u32 = 40;

/// The file name of slot `index` in `out/`: numbered in post order.
pub fn out_name(index: usize, slot: &Slot) -> String {
    let kind = match (&slot.collage, slot.role) {
        (Some(_), _) => "collage",
        (None, Role::Cover) => "cover",
        (None, Role::Body) => "photo",
    };
    format!("{:02}-{kind}.jpg", index + 1)
}

/// The AppleScript that prints the IDs of the items selected in Photos, one per line.
pub fn selection_script() -> Vec<String> {
    [
        "tell application \"Photos\"",
        "set out to \"\"",
        "repeat with m in (get selection)",
        "set out to out & (id of m) & linefeed",
        "end repeat",
        "return out",
        "end tell",
    ]
    .iter()
    .flat_map(|line| ["-e".to_string(), (*line).to_string()])
    .collect()
}

/// The IDs selected in Photos right now.
///
/// # Errors
/// `osascript` fails, or Photos returns an ID that does not validate.
pub fn selected_ids() -> Res<Vec<String>> {
    let out = run_capture("osascript", &selection_script())?;
    let ids: Vec<String> = out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    for id in &ids {
        photos_id(id)?;
    }
    if ids.is_empty() {
        return Err("nothing is selected in Photos".into());
    }
    Ok(ids)
}

/// The `osascript` argv that exports one item, with edits applied, into `dir`.
///
/// # Errors
/// The ID does not validate or the path holds a quote.
pub fn export_script(id: &str, dir: &Path) -> Res<Vec<String>> {
    photos_id(id)?;
    let dir = dir.display().to_string();
    if dir.contains('"') || dir.contains('\\') {
        return Err(format!("export folder '{dir}' holds a quote or backslash"));
    }
    Ok(vec![
        "-e".into(),
        format!("tell application \"Photos\" to export {{media item id \"{id}\"}} to (POSIX file \"{dir}\")"),
    ])
}

/// `magick` argv that resizes one exported file for upload.
pub fn resize_args(src: &Path, dst: &Path) -> Vec<String> {
    vec![
        src.display().to_string(),
        "-auto-orient".into(),
        "-resize".into(),
        format!("{LONG_EDGE}x{LONG_EDGE}>"),
        "-quality".into(),
        "90".into(),
        dst.display().to_string(),
    ]
}

/// A cell of a collage: left, top, width, height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Canvas size and the two cells of a layout.
pub fn grid(layout: Layout) -> ((u32, u32), [Cell; 2]) {
    match layout {
        Layout::Pair => {
            let w = 2160;
            let cw = (w - 2 * PAD - GAP) / 2;
            let ch = cw * 4 / 3;
            let a = Cell {
                x: PAD,
                y: PAD,
                w: cw,
                h: ch,
            };
            (
                (w, ch + 2 * PAD),
                [
                    a,
                    Cell {
                        x: PAD + cw + GAP,
                        ..a
                    },
                ],
            )
        }
        Layout::Stack => {
            let (w, h) = (2160, 2700);
            let ch = (h - 2 * PAD - GAP) / 2;
            let a = Cell {
                x: PAD,
                y: PAD,
                w: w - 2 * PAD,
                h: ch,
            };
            (
                (w, h),
                [
                    a,
                    Cell {
                        y: PAD + ch + GAP,
                        ..a
                    },
                ],
            )
        }
    }
}

fn background(accent: Accent) -> &'static str {
    match accent {
        Accent::Lowpoly => "#F4F2FB",
        Accent::None | Accent::Pixels => "#FBF5F8",
    }
}

/// Draw primitives for the two corner accents of a `w` x `h` canvas.
pub fn accent_draws(accent: Accent, w: u32, h: u32) -> Vec<(&'static str, String)> {
    match accent {
        Accent::None => Vec::new(),
        Accent::Pixels => {
            let q = 30;
            let b = h - q;
            vec![
                (
                    "#E20074",
                    format!("rectangle 0,0,{},{q} rectangle 0,{q},{q},{}", 2 * q, 2 * q),
                ),
                (
                    "#F49AC8",
                    format!(
                        "rectangle {},0,{},{q} rectangle {q},{q},{},{}",
                        2 * q,
                        3 * q,
                        2 * q,
                        2 * q
                    ),
                ),
                (
                    "#E20074",
                    format!(
                        "rectangle {},{b},{w},{h} rectangle {},{},{w},{b}",
                        w - 2 * q,
                        w - q,
                        b - q
                    ),
                ),
                (
                    "#F49AC8",
                    format!(
                        "rectangle {},{b},{},{h} rectangle {},{},{},{b}",
                        w - 3 * q,
                        w - 2 * q,
                        w - 2 * q,
                        b - q,
                        w - q
                    ),
                ),
            ]
        }
        Accent::Lowpoly => vec![
            ("#E8336E", "polygon 0,0 78,0 0,78".into()),
            ("#8A5CF6", "polygon 78,0 128,0 78,44".into()),
            ("#2E2A6B", "polygon 0,78 0,128 44,78".into()),
            ("#F59AB8", "polygon 78,0 78,44 40,40".into()),
            (
                "#E8336E",
                format!("polygon {w},{h} {},{h} {w},{}", w - 78, h - 78),
            ),
            (
                "#8A5CF6",
                format!(
                    "polygon {},{h} {},{h} {},{}",
                    w - 78,
                    w - 128,
                    w - 78,
                    h - 44
                ),
            ),
            (
                "#2E2A6B",
                format!(
                    "polygon {w},{} {w},{} {},{}",
                    h - 78,
                    h - 128,
                    w - 44,
                    h - 78
                ),
            ),
            (
                "#F59AB8",
                format!(
                    "polygon {},{h} {},{} {},{}",
                    w - 78,
                    w - 78,
                    h - 44,
                    w - 40,
                    h - 40
                ),
            ),
        ],
    }
}

/// Every `magick` argv, in order, that builds a collage from two exported photos.
pub fn collage_steps(
    collage: &Collage,
    parts: [&Path; 2],
    tmp: &Path,
    dst: &Path,
) -> Vec<Vec<String>> {
    let ((w, h), cells) = grid(collage.layout);
    let p = |name: &str| tmp.join(name).display().to_string();
    let shadow: String = cells
        .iter()
        .map(|c| {
            format!(
                "roundrectangle {},{},{},{},{RADIUS},{RADIUS} ",
                c.x,
                c.y + 14,
                c.x + c.w,
                c.y + c.h + 14
            )
        })
        .collect();
    let mut steps = vec![
        vec![
            "-size".into(),
            format!("{w}x{h}"),
            "xc:none".into(),
            "-fill".into(),
            "rgba(50,10,60,0.30)".into(),
            "-draw".into(),
            shadow,
            "-blur".into(),
            "0x22".into(),
            p("shadow.png"),
        ],
        vec![
            "-size".into(),
            format!("{w}x{h}"),
            format!("xc:{}", background(collage.accent)),
            p("shadow.png"),
            "-composite".into(),
            p("canvas.png"),
        ],
    ];
    for (i, (cell, src)) in cells.iter().zip(parts).enumerate() {
        let (cw, ch) = (cell.w, cell.h);
        steps.push(vec![
            src.display().to_string(),
            "-auto-orient".into(),
            "-resize".into(),
            format!("{cw}x{ch}^"),
            "-gravity".into(),
            "center".into(),
            "-extent".into(),
            format!("{cw}x{ch}"),
            p(&format!("cell{i}.png")),
        ]);
        steps.push(vec![
            "-size".into(),
            format!("{cw}x{ch}"),
            "xc:black".into(),
            "-fill".into(),
            "white".into(),
            "-draw".into(),
            format!("roundrectangle 0,0,{},{},{RADIUS},{RADIUS}", cw - 1, ch - 1),
            p(&format!("mask{i}.png")),
        ]);
        steps.push(vec![
            p(&format!("cell{i}.png")),
            p(&format!("mask{i}.png")),
            "-alpha".into(),
            "off".into(),
            "-compose".into(),
            "CopyOpacity".into(),
            "-composite".into(),
            p(&format!("round{i}.png")),
        ]);
        steps.push(vec![
            p("canvas.png"),
            p(&format!("round{i}.png")),
            "-geometry".into(),
            format!("+{}+{}", cell.x, cell.y),
            "-compose".into(),
            "over".into(),
            "-composite".into(),
            p("canvas.png"),
        ]);
    }
    let mut last = vec![p("canvas.png")];
    for (color, draw) in accent_draws(collage.accent, w, h) {
        last.extend(["-fill".into(), color.to_string(), "-draw".into(), draw]);
    }
    last.extend(["-quality".into(), "90".into(), dst.display().to_string()]);
    steps.push(last);
    steps
}

/// Export every slot of a post into `out/`, replacing the previous export.
///
/// # Errors
/// Photos, ImageMagick or a file operation fails. The previous `out/` stays when any slot fails.
pub fn export(slots: &[Slot], out: &Path) -> Res<Vec<PathBuf>> {
    let stage = out.with_file_name("out.staging");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let result = export_into(slots, &stage);
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(e);
    }
    let _ = std::fs::remove_dir_all(out);
    std::fs::rename(&stage, out).map_err(|e| e.to_string())?;
    Ok(slots
        .iter()
        .enumerate()
        .map(|(i, s)| out.join(out_name(i, s)))
        .collect())
}

fn export_into(slots: &[Slot], stage: &Path) -> Res<()> {
    let work = stage.join(".work");
    for (i, slot) in slots.iter().enumerate() {
        let dst = stage.join(out_name(i, slot));
        match (&slot.photos_id, &slot.collage) {
            (Some(id), _) => {
                let src = fetch(id, &work.join(format!("{i}")))?;
                run("magick", &resize_args(&src, &dst))?;
            }
            (None, Some(collage)) => {
                let a = fetch(&collage.parts[0], &work.join(format!("{i}a")))?;
                let b = fetch(&collage.parts[1], &work.join(format!("{i}b")))?;
                let tmp = work.join(format!("{i}c"));
                std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
                for step in collage_steps(collage, [&a, &b], &tmp, &dst) {
                    run("magick", &step)?;
                }
            }
            (None, None) => return Err(format!("slot {} is empty", i + 1)),
        }
    }
    std::fs::remove_dir_all(&work).map_err(|e| e.to_string())
}

/// Export one Photos item into a fresh folder and return the image file.
fn fetch(id: &str, dir: &Path) -> Res<PathBuf> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    run("osascript", &export_script(id, dir)?)?;
    std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.extension().and_then(|x| x.to_str()).is_some_and(|x| {
                matches!(
                    x.to_ascii_lowercase().as_str(),
                    "jpg" | "jpeg" | "heic" | "png"
                )
            })
        })
        .ok_or(format!("Photos exported no image for {id}"))
}

fn run(program: &str, args: &[String]) -> Res<()> {
    run_capture(program, args).map(|_| ())
}

fn run_capture(program: &str, args: &[String]) -> Res<String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grids_fit_their_canvas() {
        for layout in [Layout::Pair, Layout::Stack] {
            let ((w, h), cells) = grid(layout);
            for c in cells {
                assert!(
                    c.x + c.w + PAD <= w && c.y + c.h + PAD <= h,
                    "{layout:?} {c:?}"
                );
            }
        }
        let ((_, h), [a, _]) = grid(Layout::Pair);
        assert_eq!(a.w * 4, a.h * 3);
        assert_eq!(h, a.h + 2 * PAD);
    }

    #[test]
    fn export_script_quotes_only_validated_values() {
        let args = export_script("ABC-1/L0/001", Path::new("/tmp/x")).unwrap();
        assert!(args[1].contains("media item id \"ABC-1/L0/001\""));
        assert!(export_script("A\"B", Path::new("/tmp")).is_err());
        assert!(export_script("A", Path::new("/tmp/\"x")).is_err());
    }

    #[test]
    fn collage_ends_in_the_destination_with_accents() {
        let collage = Collage {
            layout: Layout::Pair,
            accent: Accent::Lowpoly,
            parts: vec!["A".into(), "B".into()],
        };
        let steps = collage_steps(
            &collage,
            [Path::new("a.jpg"), Path::new("b.jpg")],
            Path::new("/t"),
            Path::new("/o/02.jpg"),
        );
        assert_eq!(steps.len(), 2 + 2 * 4 + 1);
        let last = steps.last().unwrap();
        assert_eq!(last.last().unwrap(), "/o/02.jpg");
        assert_eq!(last.iter().filter(|a| *a == "-draw").count(), 8);
        assert!(steps[1][2].contains("#F4F2FB"));
    }

    #[test]
    fn out_names_follow_post_order() {
        let cover = Slot {
            photos_id: Some("A".into()),
            role: Role::Cover,
            people: vec![],
            collage: None,
        };
        assert_eq!(out_name(0, &cover), "01-cover.jpg");
        let body = Slot {
            role: Role::Body,
            ..cover
        };
        assert_eq!(out_name(9, &body), "10-photo.jpg");
    }
}
