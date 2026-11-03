//! The GitHub adapter: which shape of URL this is, and what to read for it.

use super::*;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum GitHubTarget {
    Repo {
        owner: String,
        repo: String,
    },
    Issue {
        owner: String,
        repo: String,
        number: u64,
    },
    Blob {
        owner: String,
        repo: String,
        branch: String,
        path: String,
    },
}

/// `(owner, repo)` for a GitHub *repository root* only. Deeper paths (issues,
/// pulls, blobs, a user profile) return None on purpose: the generic article
/// path already renders those readably, while the repo API below answers a
/// question no HTML strip can (description, topics, README).
#[allow(dead_code)]
pub(super) fn github_repo(url: &str) -> Option<(String, String)> {
    if host_of(url) != "github.com" && host_of(url) != "www.github.com" {
        return None;
    }
    let seg = path_segments(url);
    if seg.len() != 2 {
        return None;
    }
    let repo = seg[1].trim_end_matches(".git");
    if seg[0].is_empty() || repo.is_empty() {
        return None;
    }
    Some((seg[0].to_string(), repo.to_string()))
}

pub(super) fn parse_github_url(url: &str) -> Option<GitHubTarget> {
    let host = host_of(url);
    if host != "github.com" && host != "www.github.com" {
        return None;
    }
    let seg = path_segments(url);
    if seg.len() < 2 {
        return None;
    }
    let owner = seg[0].to_string();
    let repo = seg[1].trim_end_matches(".git").to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }

    if seg.len() == 2 {
        return Some(GitHubTarget::Repo { owner, repo });
    }

    if (seg[2] == "issues" || seg[2] == "pull") && seg.len() >= 4 {
        if let Ok(number) = seg[3].parse::<u64>() {
            return Some(GitHubTarget::Issue {
                owner,
                repo,
                number,
            });
        }
    }

    if (seg[2] == "blob" || seg[2] == "raw") && seg.len() >= 5 {
        let branch = seg[3].to_string();
        let path = seg[4..].join("/");
        return Some(GitHubTarget::Blob {
            owner,
            repo,
            branch,
            path,
        });
    }

    Some(GitHubTarget::Repo { owner, repo })
}

/// A GitHub repository: description, topics and README. The README is the
/// transcript because it is the part worth summarizing; the metadata line above
/// it is what the repo page shows and a README often omits.
pub(super) fn fetch_github(
    owner: &str,
    repo: &str,
) -> Result<(Option<String>, Option<String>, String)> {
    let http = http_client()?;
    let meta = get_json(
        &http,
        &format!("https://api.github.com/repos/{owner}/{repo}"),
        "application/vnd.github+json",
    )?;

    let full_name = meta
        .get("full_name")
        .and_then(|v| v.as_str())
        .unwrap_or(repo);
    let description = meta
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let title = Some(if description.is_empty() {
        full_name.to_string()
    } else {
        format!("{full_name} — {description}")
    });

    let mut head = String::new();
    if !description.is_empty() {
        head.push_str(description);
        head.push('\n');
    }
    let stars = meta
        .get("stargazers_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let language = meta.get("language").and_then(|v| v.as_str()).unwrap_or("—");
    let license = meta
        .get("license")
        .and_then(|l| l.get("spdx_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("—");
    head.push_str(&format!(
        "Stars: {stars} · Language: {language} · License: {license}\n"
    ));
    if let Some(topics) = meta.get("topics").and_then(|v| v.as_array()) {
        let list: Vec<&str> = topics.iter().filter_map(|t| t.as_str()).collect();
        if !list.is_empty() {
            head.push_str(&format!("Topics: {}\n", list.join(", ")));
        }
    }

    // The raw README, when there is one. A repo without one still ingests.
    let readme = http
        .get(format!(
            "https://api.github.com/repos/{owner}/{repo}/readme"
        ))
        .header("Accept", "application/vnd.github.raw")
        .send()
        .ok()
        .filter(|r| r.status().is_success())
        .and_then(|r| r.text().ok())
        .unwrap_or_default();

    let author = meta
        .get("owner")
        .and_then(|o| o.get("login"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    Ok((
        title,
        author,
        cap_text(format!("{head}\n{readme}").trim().to_string()),
    ))
}

pub(super) fn fetch_github_target(
    target: &GitHubTarget,
    url: &str,
) -> Result<(Option<String>, Option<String>, String)> {
    match target {
        GitHubTarget::Repo { owner, repo } => fetch_github(owner, repo),
        GitHubTarget::Issue {
            owner,
            repo,
            number,
        } => {
            let http = http_client()?;
            let issue_res = get_json(
                &http,
                &format!("https://api.github.com/repos/{owner}/{repo}/issues/{number}"),
                "application/vnd.github+json",
            );
            if let Ok(issue) = issue_res {
                let title_text = issue.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let body_text = issue.get("body").and_then(|v| v.as_str()).unwrap_or("");
                let author = issue
                    .get("user")
                    .and_then(|u| u.get("login"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let title = Some(format!("{owner}/{repo}#{number}: {title_text}"));
                let mut text = format!("# {title_text}\n\n{body_text}");

                if let Ok(comments) = get_json(
                    &http,
                    &format!("https://api.github.com/repos/{owner}/{repo}/issues/{number}/comments?per_page=10"),
                    "application/vnd.github+json",
                ) {
                    if let Some(arr) = comments.as_array() {
                        for c in arr {
                            let user = c.get("user").and_then(|u| u.get("login")).and_then(|v| v.as_str()).unwrap_or("?");
                            let comment_body = c.get("body").and_then(|v| v.as_str()).unwrap_or("").trim();
                            if !comment_body.is_empty() {
                                text.push_str(&format!("\n\n---\n**@{user}**:\n{comment_body}"));
                            }
                        }
                    }
                }
                return Ok((title, author, cap_text(text)));
            }
            extract_article(url).map(|(t, text)| (t, Some(owner.clone()), text))
        }
        GitHubTarget::Blob {
            owner,
            repo,
            branch,
            path,
        } => {
            let http = http_client()?;
            let raw_url =
                format!("https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{path}");
            let resp = http.get(&raw_url).send();
            if let Ok(r) = resp {
                if r.status().is_success() {
                    if let Ok(text) = r.text() {
                        let title = Some(format!("{path} ({owner}/{repo})"));
                        return Ok((title, Some(owner.clone()), cap_text(text)));
                    }
                }
            }
            extract_article(url).map(|(t, text)| (t, Some(owner.clone()), text))
        }
    }
}
