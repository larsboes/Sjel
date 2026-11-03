//! The Hugging Face adapter: model, dataset, space or paper.

use super::*;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum HuggingFaceTarget {
    Model { model_id: String },
    Dataset { dataset_id: String },
    Paper { paper_id: String },
}

pub(super) fn parse_huggingface_url(url: &str) -> Option<HuggingFaceTarget> {
    let host = host_of(url);
    if !host.ends_with("huggingface.co") {
        return None;
    }
    let seg = path_segments(url);
    if seg.is_empty() {
        return None;
    }

    if seg[0] == "papers" && seg.len() >= 2 {
        return Some(HuggingFaceTarget::Paper {
            paper_id: seg[1].to_string(),
        });
    }

    if seg[0] == "datasets" && seg.len() >= 2 {
        let dataset_id = if seg.len() >= 3 {
            format!("{}/{}", seg[1], seg[2])
        } else {
            seg[1].to_string()
        };
        return Some(HuggingFaceTarget::Dataset { dataset_id });
    }

    if seg.len() >= 2 {
        let model_id = format!("{}/{}", seg[0], seg[1]);
        return Some(HuggingFaceTarget::Model { model_id });
    } else if seg.len() == 1 && !seg[0].contains('/') {
        return Some(HuggingFaceTarget::Model {
            model_id: seg[0].to_string(),
        });
    }

    None
}

/// Returns the arXiv shape, because one of its arms IS arXiv: a Hugging Face
/// paper page is a view of the same preprint, so whether the text is the paper
/// or its abstract is a fact about this item too, not one arXiv URLs alone get
/// to carry (#78). Every other arm reads the thing itself.
pub(super) fn fetch_huggingface(
    target: &HuggingFaceTarget,
    url: &str,
) -> Result<(Option<String>, Option<String>, String, TranscriptSource)> {
    match target {
        HuggingFaceTarget::Paper { paper_id } => fetch_arxiv(paper_id),
        HuggingFaceTarget::Model { model_id } => {
            let http = http_client()?;
            let meta_res = get_json(
                &http,
                &format!("https://huggingface.co/api/models/{model_id}"),
                "application/json",
            );
            let raw_readme = http
                .get(format!(
                    "https://huggingface.co/{model_id}/raw/main/README.md"
                ))
                .send()
                .ok()
                .filter(|r| r.status().is_success())
                .and_then(|r| r.text().ok())
                .unwrap_or_default();

            let author = model_id.split_once('/').map(|(a, _)| a.to_string());
            if let Ok(meta) = meta_res {
                let pipeline = meta
                    .get("pipeline_tag")
                    .and_then(|v| v.as_str())
                    .unwrap_or("model");
                let downloads = meta.get("downloads").and_then(|v| v.as_u64()).unwrap_or(0);
                let likes = meta.get("likes").and_then(|v| v.as_u64()).unwrap_or(0);
                let title = Some(format!("{model_id} ({pipeline})"));

                let header = format!("Model: {model_id} · Pipeline: {pipeline} · Downloads: {downloads} · Likes: {likes}");
                let text = format!("{header}\n\n---\n\n{raw_readme}");
                return Ok((title, author, cap_text(text), TranscriptSource::FullText));
            }

            extract_article(url).map(|(t, text)| (t, author, text, TranscriptSource::FullText))
        }
        HuggingFaceTarget::Dataset { dataset_id } => {
            let http = http_client()?;
            let meta_res = get_json(
                &http,
                &format!("https://huggingface.co/api/datasets/{dataset_id}"),
                "application/json",
            );
            let raw_readme = http
                .get(format!(
                    "https://huggingface.co/datasets/{dataset_id}/raw/main/README.md"
                ))
                .send()
                .ok()
                .filter(|r| r.status().is_success())
                .and_then(|r| r.text().ok())
                .unwrap_or_default();

            let author = dataset_id.split_once('/').map(|(a, _)| a.to_string());
            if let Ok(meta) = meta_res {
                let downloads = meta.get("downloads").and_then(|v| v.as_u64()).unwrap_or(0);
                let likes = meta.get("likes").and_then(|v| v.as_u64()).unwrap_or(0);
                let title = Some(format!("Dataset: {dataset_id}"));

                let header =
                    format!("Dataset: {dataset_id} · Downloads: {downloads} · Likes: {likes}");
                let text = format!("{header}\n\n---\n\n{raw_readme}");
                return Ok((title, author, cap_text(text), TranscriptSource::FullText));
            }

            extract_article(url).map(|(t, text)| (t, author, text, TranscriptSource::FullText))
        }
    }
}
