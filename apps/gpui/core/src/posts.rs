//! Post drafts (`/posts`). Backed by `crates/posts/posts` through `GET` and
//! `POST /api/sjel-status/posts` and `GET /api/sjel-status/posts/detail`.

pub use status_api::{
    PostCollage, PostDetail, PostMediaItem, PostSummary, PostsView, PostsWriteRequest,
    PostsWriteResult,
};

/// The states a post moves through, in order.
pub const STATES: [&str; 4] = ["sketch", "draft", "ready", "published"];

/// The accent that follows `current` when the operator cycles a collage's accent.
pub fn next_accent(current: &str) -> &'static str {
    match current {
        "none" => "pixels",
        "pixels" => "lowpoly",
        _ => "none",
    }
}

/// A short label for a media slot: its number, kind and role.
pub fn slot_label(index: usize, item: &PostMediaItem) -> String {
    match &item.collage {
        Some(c) => format!("{}. Collage ({}, {})", index + 1, c.layout, c.accent),
        None => format!("{}. Photo ({})", index + 1, item.role),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accents_cycle_through_all_three() {
        assert_eq!(next_accent("none"), "pixels");
        assert_eq!(next_accent("pixels"), "lowpoly");
        assert_eq!(next_accent("lowpoly"), "none");
    }

    #[test]
    fn labels_name_number_and_kind() {
        let photo = PostMediaItem {
            role: "cover".into(),
            ..Default::default()
        };
        assert_eq!(slot_label(0, &photo), "1. Photo (cover)");
        let collage = PostMediaItem {
            collage: Some(PostCollage {
                layout: "pair".into(),
                accent: "lowpoly".into(),
                parts: vec![],
            }),
            ..Default::default()
        };
        assert_eq!(slot_label(6, &collage), "7. Collage (pair, lowpoly)");
    }
}
