//! Unicode61 token-prefix matching for the portable document search.

#[path = "unicode61_tables.rs"]
mod tables;

fn class(character: char) -> u8 {
    let character = u32::from(character);
    let index = tables::CLASSES.partition_point(|&(_, end, _)| end < character);
    tables::CLASSES.get(index).map_or(
        0,
        |&(start, _, kind)| {
            if start <= character {
                kind
            } else {
                0
            }
        },
    )
}

fn fold(character: char) -> Option<char> {
    let code = u32::from(character);
    match tables::FOLDS.binary_search_by_key(&code, |&(from, _)| from) {
        Ok(index) => {
            let mapped = tables::FOLDS[index].1;
            if mapped == 0 {
                None
            } else {
                char::from_u32(mapped)
            }
        }
        Err(_) => Some(character),
    }
}

fn tokens(text: &str) -> impl Iterator<Item = Vec<u8>> + '_ {
    let mut chars = text.chars();
    std::iter::from_fn(move || {
        let start = chars.find(|&character| class(character) == 1)?;
        let rest = chars
            .by_ref()
            .take_while(|&character| class(character) != 0);
        let mut normalized = std::iter::once(start)
            .chain(rest)
            .filter_map(fold)
            .collect::<String>()
            .into_bytes();
        normalized.truncate(32_768);
        Some(normalized)
    })
}

pub(super) struct Prefixes(Vec<Vec<Vec<u8>>>);

impl Prefixes {
    pub(super) fn new(words: &[String]) -> Self {
        Self(
            words
                .iter()
                .map(|word| tokens(word).collect::<Vec<_>>())
                .filter(|phrase| !phrase.is_empty())
                .collect(),
        )
    }

    pub(super) fn matches(&self, text: &str) -> bool {
        let mut input = tokens(text);
        if self.0.iter().all(|phrase| phrase.len() == 1) {
            return input.any(|token| self.0.iter().any(|phrase| token.starts_with(&phrase[0])));
        }
        let tokens: Vec<_> = input.collect();
        self.0.iter().any(|phrase| {
            tokens.windows(phrase.len()).any(|window| {
                let last = phrase.len() - 1;
                window[..last] == phrase[..last] && window[last].starts_with(&phrase[last])
            })
        })
    }
}

#[cfg(all(test, not(feature = "turso-wal")))]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn preserves_sqlite_unicode61_prefix_candidates() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE VIRTUAL TABLE control USING fts5(text, tokenize='unicode61 remove_diacritics 2')").unwrap();
        let fixtures = [
            ("faḧrgast Rechte", &["fahr", "gast", "rechte"][..]),
            ("MÜNCHEN cafÉ", &["mün", "mun", "cafe"]),
            (
                "Ελλάδα Ελληνικά Straße ὀδός",
                &["ελλ", "ελλα", "stra", "οδο"],
            ),
            (
                "𝔸123 Straße ﬁling αβ-123_456",
                &["𝔸1", "123", "stras", "ﬁli", "456"],
            ),
            (
                "cafe\u{301} ộ\u{302} drei\u{0}vier",
                &["café", "caf", "drei", "vier", "o"],
            ),
            ("a\u{1c90}z", &["a\u{1c90}z", "a", "z"]),
        ];
        for (text, words) in fixtures {
            conn.execute("INSERT INTO control(rowid, text) VALUES (1, ?1)", [text])
                .unwrap();
            for word in words {
                let control: bool = conn
                    .query_row(
                        "SELECT EXISTS (SELECT 1 FROM control WHERE control MATCH ?1)",
                        [format!("{word}*")],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(
                    Prefixes::new(&[word.to_string()]).matches(text),
                    control,
                    "{word} in {text:?}"
                );
            }
            conn.execute("DELETE FROM control", []).unwrap();
        }
    }
}
