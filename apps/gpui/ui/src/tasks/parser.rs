/// A due date as the table accepts it: empty, or `YYYY-MM-DD`.
pub fn valid_due(due: &str) -> bool {
    due.is_empty() || chrono::NaiveDate::parse_from_str(due, "%Y-%m-%d").is_ok()
}

/// Extracts subtask completion from markdown checklist items (`- [ ]` and `- [x]`).
pub fn parse_subtask_progress(content: &str) -> Option<(usize, usize)> {
    let mut done = 0;
    let mut total = 0;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("- [x]")
            || trimmed.starts_with("- [X]")
            || trimmed.starts_with("* [x]")
            || trimmed.starts_with("* [X]")
        {
            done += 1;
            total += 1;
        } else if trimmed.starts_with("- [ ]") || trimmed.starts_with("* [ ]") {
            total += 1;
        }
    }
    if total > 0 {
        Some((done, total))
    } else {
        None
    }
}

/// Parses inline syntax from a task entry string:
/// `!1`, `!2`, `!3` for priority,
/// `@today`, `@tomorrow`, `@YYYY-MM-DD` for due date.
/// Returns (clean_title, due_date, priority).
pub fn parse_smart_task_input(
    raw: &str,
    today: chrono::NaiveDate,
) -> (String, Option<String>, Option<u8>) {
    let mut due = None;
    let mut priority = None;
    let mut words = Vec::new();

    for word in raw.split_whitespace() {
        if word == "!1" {
            priority = Some(1);
        } else if word == "!2" {
            priority = Some(2);
        } else if word == "!3" {
            priority = Some(3);
        } else if word.eq_ignore_ascii_case("@today") {
            due = Some(today.format("%Y-%m-%d").to_string());
        } else if word.eq_ignore_ascii_case("@tomorrow") {
            due = Some(
                (today + chrono::Duration::days(1))
                    .format("%Y-%m-%d")
                    .to_string(),
            );
        } else if let Some(date_part) = word.strip_prefix('@') {
            if chrono::NaiveDate::parse_from_str(date_part, "%Y-%m-%d").is_ok() {
                due = Some(date_part.to_string());
            } else {
                words.push(word);
            }
        } else {
            words.push(word);
        }
    }

    (words.join(" "), due, priority)
}
