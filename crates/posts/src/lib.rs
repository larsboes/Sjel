//! Post drafts under `<overlay>/data/posts/<slug>/`. The file contract is
//! `docs/architecture/posts.md`: `post.md` (TOML front matter and text), `revisions/`,
//! `media.toml` (ordered slots) and `out/` (numbered exports).

pub mod media;

use serde::{Deserialize, Serialize};
use status_api::{PostCollage, PostDetail, PostMediaItem, PostSummary, PostsView};
use std::fs;
use std::path::{Path, PathBuf};

pub type Res<T> = Result<T, String>;

const FENCE: &str = "+++";

/// The lifecycle of one post.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Sketch,
    Draft,
    Ready,
    Published,
}

impl State {
    /// # Errors
    /// The name is not one of the four states.
    pub fn parse(name: &str) -> Res<State> {
        match name {
            "sketch" => Ok(State::Sketch),
            "draft" => Ok(State::Draft),
            "ready" => Ok(State::Ready),
            "published" => Ok(State::Published),
            other => Err(format!(
                "state '{other}' is not sketch, draft, ready or published"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            State::Sketch => "sketch",
            State::Draft => "draft",
            State::Ready => "ready",
            State::Published => "published",
        }
    }
}

/// The front matter of `post.md`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    pub title: String,
    pub state: State,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// What a slot shows in the post.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Cover,
    Body,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    /// Two portrait photos side by side.
    Pair,
    /// Two landscape photos, one above the other.
    Stack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Accent {
    None,
    /// Magenta pixel squares in two corners.
    Pixels,
    /// Low-poly triangles in two corners.
    Lowpoly,
}

impl Layout {
    /// # Errors
    /// Unknown layout name.
    pub fn parse(name: &str) -> Res<Layout> {
        match name {
            "pair" => Ok(Layout::Pair),
            "stack" => Ok(Layout::Stack),
            other => Err(format!("layout '{other}' is not pair or stack")),
        }
    }
}

impl Accent {
    /// # Errors
    /// Unknown accent name.
    pub fn parse(name: &str) -> Res<Accent> {
        match name {
            "none" | "" => Ok(Accent::None),
            "pixels" => Ok(Accent::Pixels),
            "lowpoly" => Ok(Accent::Lowpoly),
            other => Err(format!("accent '{other}' is not none, pixels or lowpoly")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Collage {
    pub layout: Layout,
    pub accent: Accent,
    pub parts: Vec<String>,
}

/// One slot of `media.toml`: exactly one of `photos_id` and `collage`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photos_id: Option<String>,
    pub role: Role,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collage: Option<Collage>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Media {
    #[serde(default, rename = "item")]
    pub items: Vec<Slot>,
}

impl Media {
    /// # Errors
    /// A slot has both or neither of `photos_id` and `collage`, a collage does not have two
    /// parts, or an ID holds characters a Photos ID does not.
    pub fn validate(&self) -> Res<()> {
        for (i, slot) in self.items.iter().enumerate() {
            match (&slot.photos_id, &slot.collage) {
                (Some(id), None) => photos_id(id)?,
                (None, Some(c)) => {
                    if c.parts.len() != 2 {
                        return Err(format!("slot {}: a collage has exactly two parts", i + 1));
                    }
                    for id in &c.parts {
                        photos_id(id)?;
                    }
                }
                _ => {
                    return Err(format!(
                        "slot {}: set exactly one of photos_id and collage",
                        i + 1
                    ))
                }
            }
        }
        Ok(())
    }

    /// Move slot `index` one place `up` or `down`.
    ///
    /// # Errors
    /// The index or the move leaves the list.
    pub fn shift(&mut self, index: usize, direction: &str) -> Res<()> {
        let target = match direction {
            "up" if index > 0 => index - 1,
            "down" => index + 1,
            "up" => return Err("slot 1 is already first".into()),
            other => return Err(format!("direction '{other}' is not up or down")),
        };
        if index >= self.items.len() || target >= self.items.len() {
            return Err(format!("slot {} cannot move {direction}", index + 1));
        }
        self.items.swap(index, target);
        Ok(())
    }

    /// Merge slot `index` and the next one into a collage. Both must be single photos.
    ///
    /// # Errors
    /// No next slot, or one of them is already a collage.
    pub fn pair(&mut self, index: usize, layout: Layout, accent: Accent) -> Res<()> {
        if index + 1 >= self.items.len() {
            return Err(format!("slot {} has no next slot to pair with", index + 1));
        }
        let (Some(a), Some(b)) = (
            self.items[index].photos_id.clone(),
            self.items[index + 1].photos_id.clone(),
        ) else {
            return Err("only two single photos can become a collage".into());
        };
        let mut people = self.items[index].people.clone();
        people.extend(self.items[index + 1].people.clone());
        self.items[index] = Slot {
            photos_id: None,
            role: self.items[index].role,
            people,
            collage: Some(Collage {
                layout,
                accent,
                parts: vec![a, b],
            }),
        };
        self.items.remove(index + 1);
        Ok(())
    }

    /// Split the collage at `index` back into its two photos.
    ///
    /// # Errors
    /// The slot is not a collage.
    pub fn split(&mut self, index: usize) -> Res<()> {
        let slot = self
            .items
            .get(index)
            .ok_or(format!("no slot {}", index + 1))?;
        let Some(collage) = slot.collage.clone() else {
            return Err(format!("slot {} is not a collage", index + 1));
        };
        let role = slot.role;
        let singles = collage.parts.into_iter().map(|id| Slot {
            photos_id: Some(id),
            role,
            people: Vec::new(),
            collage: None,
        });
        self.items.splice(index..=index, singles);
        Ok(())
    }
}

/// # Errors
/// The ID is empty or holds a character outside `[A-Za-z0-9/-]`.
pub fn photos_id(id: &str) -> Res<()> {
    if !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '-')
    {
        Ok(())
    } else {
        Err(format!("'{id}' is not a Photos media item ID"))
    }
}

/// # Errors
/// The slug is not lowercase ASCII letters, digits and single dashes.
pub fn slug(name: &str) -> Res<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--");
    if ok {
        Ok(())
    } else {
        Err(format!(
            "slug '{name}' must be lowercase letters, digits and single dashes"
        ))
    }
}

/// Split `post.md` into front matter and text.
///
/// # Errors
/// The fences are missing or the front matter does not parse.
pub fn parse_post(raw: &str) -> Res<(Meta, String)> {
    let rest = raw
        .strip_prefix(FENCE)
        .and_then(|r| r.strip_prefix('\n'))
        .ok_or("post.md must start with a +++ line")?;
    let end = rest
        .find("\n+++")
        .ok_or("post.md has no closing +++ line")?;
    let meta: Meta = toml::from_str(&rest[..end]).map_err(|e| format!("front matter: {e}"))?;
    let body = rest[end + 4..]
        .strip_prefix('\n')
        .unwrap_or(&rest[end + 4..]);
    Ok((meta, body.to_string()))
}

/// # Errors
/// The front matter does not serialize.
pub fn render_post(meta: &Meta, text: &str) -> Res<String> {
    let front = toml::to_string(meta).map_err(|e| e.to_string())?;
    Ok(format!("{FENCE}\n{front}{FENCE}\n{text}"))
}

/// The posts folder of the active overlay.
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    /// Resolve `<overlay>/data/posts` from `SJEL_PERSONAL_ROOT`.
    ///
    /// # Errors
    /// The variable is unset.
    pub fn from_env() -> Res<Store> {
        let overlay = sjel_config::overlay_root().ok_or("SJEL_PERSONAL_ROOT is not set")?;
        Ok(Store {
            root: overlay.join("data/posts"),
        })
    }

    pub fn dir(&self, slug: &str) -> PathBuf {
        self.root.join(slug)
    }

    pub fn out_dir(&self, slug: &str) -> PathBuf {
        self.dir(slug).join("out")
    }

    /// # Errors
    /// Invalid slug, or the folder already exists.
    pub fn create(&self, name: &str, title: &str, event: Option<String>) -> Res<()> {
        slug(name)?;
        let dir = self.dir(name);
        if dir.exists() {
            return Err(format!("post '{name}' already exists"));
        }
        fs::create_dir_all(dir.join("revisions")).map_err(io)?;
        fs::create_dir_all(dir.join("out")).map_err(io)?;
        let meta = Meta {
            title: title.to_string(),
            state: State::Sketch,
            event,
            url: None,
        };
        write(&dir.join("post.md"), &render_post(&meta, "")?)?;
        self.save_media(name, &Media::default())
    }

    /// # Errors
    /// The post does not exist or `post.md` does not parse.
    pub fn read(&self, name: &str) -> Res<(Meta, String)> {
        slug(name)?;
        let raw = fs::read_to_string(self.dir(name).join("post.md"))
            .map_err(|e| format!("post '{name}': {e}"))?;
        parse_post(&raw)
    }

    /// Replace the text and keep the previous text as the next numbered revision.
    ///
    /// # Errors
    /// The post does not exist or a write fails.
    pub fn set_text(&self, name: &str, text: &str) -> Res<usize> {
        let (meta, old) = self.read(name)?;
        let revisions = self.dir(name).join("revisions");
        fs::create_dir_all(&revisions).map_err(io)?;
        let number = self.revisions(name) + 1;
        write(&revisions.join(format!("{number:03}.md")), &old)?;
        write(&self.dir(name).join("post.md"), &render_post(&meta, text)?)?;
        Ok(number)
    }

    /// # Errors
    /// The post does not exist or a write fails.
    pub fn set_state(&self, name: &str, state: State, url: Option<String>) -> Res<()> {
        let (mut meta, text) = self.read(name)?;
        meta.state = state;
        if url.is_some() {
            meta.url = url;
        }
        write(&self.dir(name).join("post.md"), &render_post(&meta, &text)?)
    }

    pub fn revisions(&self, name: &str) -> usize {
        fs::read_dir(self.dir(name).join("revisions"))
            .map(|d| d.filter_map(Result::ok).count())
            .unwrap_or(0)
    }

    /// # Errors
    /// `media.toml` does not parse or fails validation.
    pub fn media(&self, name: &str) -> Res<Media> {
        let path = self.dir(name).join("media.toml");
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Media::default()),
            Err(e) => return Err(io(e)),
        };
        let media: Media = toml::from_str(&raw).map_err(|e| format!("media.toml: {e}"))?;
        media.validate()?;
        Ok(media)
    }

    /// # Errors
    /// Validation or the write fails.
    pub fn save_media(&self, name: &str, media: &Media) -> Res<()> {
        media.validate()?;
        let raw = toml::to_string(media).map_err(|e| e.to_string())?;
        write(&self.dir(name).join("media.toml"), &raw)
    }

    /// Every post folder, sorted by slug.
    ///
    /// # Errors
    /// A post folder holds a `post.md` that does not parse.
    pub fn view(&self) -> Res<PostsView> {
        let mut posts = Vec::new();
        if let Ok(entries) = fs::read_dir(&self.root) {
            let mut names: Vec<String> = entries
                .filter_map(Result::ok)
                .filter(|e| e.path().join("post.md").is_file())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect();
            names.sort();
            for name in names {
                posts.push(self.summary(&name)?);
            }
        }
        Ok(PostsView {
            root: self.root.display().to_string(),
            posts,
        })
    }

    fn summary(&self, name: &str) -> Res<PostSummary> {
        let (meta, _) = self.read(name)?;
        Ok(PostSummary {
            slug: name.to_string(),
            title: meta.title,
            state: meta.state.name().to_string(),
            event: meta.event,
            url: meta.url,
            media: self.media(name)?.items.len(),
        })
    }

    /// # Errors
    /// The post does not exist or does not parse.
    pub fn detail(&self, name: &str) -> Res<PostDetail> {
        let (_, text) = self.read(name)?;
        let media = self.media(name)?;
        let out = self.out_dir(name);
        let items = media
            .items
            .iter()
            .enumerate()
            .map(|(i, slot)| {
                let file = out.join(media::out_name(i, slot));
                PostMediaItem {
                    photos_id: slot.photos_id.clone().unwrap_or_default(),
                    role: match slot.role {
                        Role::Cover => "cover".into(),
                        Role::Body => "body".into(),
                    },
                    people: slot.people.clone(),
                    collage: slot.collage.as_ref().map(|c| PostCollage {
                        layout: format!("{:?}", c.layout).to_lowercase(),
                        accent: format!("{:?}", c.accent).to_lowercase(),
                        parts: c.parts.clone(),
                    }),
                    file: file.is_file().then(|| file.display().to_string()),
                }
            })
            .collect();
        Ok(PostDetail {
            summary: self.summary(name)?,
            text,
            media: items,
            revisions: self.revisions(name),
        })
    }
}

fn io(e: std::io::Error) -> String {
    e.to_string()
}

fn write(path: &Path, content: &str) -> Res<()> {
    fs::write(path, content).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single(id: &str) -> Slot {
        Slot {
            photos_id: Some(id.into()),
            role: Role::Body,
            people: vec![],
            collage: None,
        }
    }

    fn scratch(tag: &str) -> Store {
        let root = std::env::temp_dir().join(format!("sjel-posts-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        Store { root }
    }

    #[test]
    fn post_round_trips_through_front_matter() {
        let meta = Meta {
            title: "Event recap".into(),
            state: State::Draft,
            event: Some("Expo".into()),
            url: None,
        };
        let text = "Line one\n\n+++ not a fence in text\n";
        let raw = render_post(&meta, text).unwrap();
        assert_eq!(parse_post(&raw).unwrap(), (meta, text.to_string()));
    }

    #[test]
    fn unknown_front_matter_key_is_refused() {
        let raw = "+++\ntitle = \"x\"\nstate = \"draft\"\nmood = \"happy\"\n+++\n";
        assert!(parse_post(raw).is_err());
    }

    #[test]
    fn slugs_and_ids_are_checked() {
        assert!(slug("explor-day-2026").is_ok());
        for bad in ["", "Caps", "a--b", "-a", "a/b", "a b"] {
            assert!(slug(bad).is_err(), "{bad}");
        }
        assert!(photos_id("ABC-123/L0/001").is_ok());
        assert!(photos_id("x\"; do shell script").is_err());
    }

    #[test]
    fn shift_pair_and_split_keep_order() {
        let mut media = Media {
            items: vec![single("A"), single("B"), single("C")],
        };
        media.shift(2, "up").unwrap();
        assert_eq!(media.items[1].photos_id.as_deref(), Some("C"));
        assert!(media.shift(0, "up").is_err());
        assert!(media.shift(2, "down").is_err());
        media.pair(1, Layout::Pair, Accent::Lowpoly).unwrap();
        assert_eq!(media.items.len(), 2);
        assert_eq!(
            media.items[1].collage.as_ref().unwrap().parts,
            vec!["C", "B"]
        );
        assert!(media.pair(0, Layout::Pair, Accent::None).is_err());
        media.split(1).unwrap();
        let ids: Vec<_> = media
            .items
            .iter()
            .map(|s| s.photos_id.clone().unwrap())
            .collect();
        assert_eq!(ids, vec!["A", "C", "B"]);
    }

    #[test]
    fn a_slot_needs_exactly_one_source() {
        let both = Slot {
            collage: Some(Collage {
                layout: Layout::Pair,
                accent: Accent::None,
                parts: vec!["A".into(), "B".into()],
            }),
            ..single("A")
        };
        assert!(Media { items: vec![both] }.validate().is_err());
    }

    #[test]
    fn store_creates_revises_and_lists() {
        let store = scratch("store");
        store.create("recap", "Recap", None).unwrap();
        assert!(store.create("recap", "Again", None).is_err());
        assert_eq!(store.set_text("recap", "first").unwrap(), 1);
        assert_eq!(store.set_text("recap", "second").unwrap(), 2);
        assert_eq!(store.read("recap").unwrap().1, "second");
        store
            .set_state(
                "recap",
                State::Published,
                Some("https://example.org/p".into()),
            )
            .unwrap();
        let view = store.view().unwrap();
        assert_eq!(view.posts.len(), 1);
        assert_eq!(view.posts[0].state, "published");
        assert_eq!(store.detail("recap").unwrap().revisions, 2);
        let _ = fs::remove_dir_all(&store.root);
    }
}
