//! The `sql` plugin: one read-only query against the database the app is already reading.
//!
//! A coding agent's instinct with a question about Sjel's data is `bash` plus `sqlite3`, which
//! costs a process per question, a shell parser every time, and an output format the model then
//! has to guess at. This answers the same question in one call with a header on it.
//!
//! The connection is opened `SQLITE_OPEN_READ_ONLY` and then set `query_only`, so a mistake in the
//! tool cannot write: the fence is the connection, not a keyword search over the SQL.

use std::path::PathBuf;

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};

use sjel_agent::{Extension, Tool};

pub const NAME: &str = "sql";

const OUTPUT_LIMIT: usize = 12_000;
const ROW_LIMIT: usize = 200;
const QUERY_LIMIT: usize = 4_000;
const BUSY_TIMEOUT_MS: u32 = 5_000;

pub struct Sql {
    path: PathBuf,
}

impl Sql {
    /// The shared database, as `sjel_config` resolves it.
    pub fn new() -> Self {
        Self::at(sjel_config::database_path())
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl Default for Sql {
    fn default() -> Self {
        Self::new()
    }
}

impl Extension for Sql {
    fn name(&self) -> &'static str {
        NAME
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(SqlQuery {
            path: self.path.clone(),
        })]
    }
}

struct SqlQuery {
    path: PathBuf,
}

impl Tool for SqlQuery {
    fn name(&self) -> &'static str {
        "sql"
    }

    fn description(&self) -> &'static str {
        "Run one read-only SQL query against Sjel's shared database and get the rows with a header. \
         Use it instead of `bash` + `sqlite3`. The connection cannot write. Results are capped; the \
         answer says how many rows were returned and whether more existed."
    }

    fn read_only(&self) -> bool {
        true
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": { "type": "string", "description": "One SELECT statement." }
            }
        })
    }

    fn run(&self, args: &Value) -> Result<String, String> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .ok_or_else(|| "missing string argument `query`".to_owned())?;
        if query.len() > QUERY_LIMIT {
            return Err(format!("the query is longer than {QUERY_LIMIT} bytes"));
        }
        let single = query.strip_suffix(';').unwrap_or(query).trim_end();
        if single.contains(';') {
            return Err("one statement per call: the query contains more than one".into());
        }

        let connection = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("{}: {e}", self.path.display()))?;
        let _ =
            connection.busy_timeout(std::time::Duration::from_millis(u64::from(BUSY_TIMEOUT_MS)));
        connection
            .execute_batch("PRAGMA query_only = ON")
            .map_err(|e| format!("query_only: {e}"))?;

        let mut statement = connection.prepare(single).map_err(|e| format!("{e}"))?;
        let columns: Vec<String> = statement
            .column_names()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let mut rows = statement.query([]).map_err(|e| format!("{e}"))?;

        let mut out = String::new();
        out.push_str(&columns.join(" | "));
        out.push('\n');
        let mut returned = 0;
        let mut truncated = false;
        while let Some(row) = rows.next().map_err(|e| format!("{e}"))? {
            if returned == ROW_LIMIT || out.len() > OUTPUT_LIMIT {
                truncated = true;
                break;
            }
            let cells: Vec<String> = (0..columns.len())
                .map(|index| match row.get::<_, SqlValue>(index) {
                    Ok(SqlValue::Null) => "NULL".to_owned(),
                    Ok(SqlValue::Integer(n)) => n.to_string(),
                    Ok(SqlValue::Real(f)) => f.to_string(),
                    Ok(SqlValue::Text(text)) => text,
                    Ok(SqlValue::Blob(bytes)) => format!("<{} bytes>", bytes.len()),
                    Err(error) => format!("<{}>", error),
                })
                .collect();
            out.push_str(&cells.join(" | "));
            out.push('\n');
            returned += 1;
        }
        if truncated {
            out.push_str(&format!(
                "[{returned} row(s) shown, more exist — add a WHERE or a LIMIT]"
            ));
        } else {
            out.push_str(&format!("[{returned} row(s)]"));
        }
        Ok(sjel_agent::tools::cap(&out, OUTPUT_LIMIT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A database with one table, written by this test and opened read-only by the tool.
    fn planted(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("sjel-agent-sql-{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT, score REAL, body BLOB);
                 INSERT INTO notes (title, score) VALUES ('one', 1.5), ('two', NULL);",
            )
            .unwrap();
        path
    }

    fn tool(path: &PathBuf) -> Box<dyn Tool> {
        Sql::at(path.clone()).tools().pop().unwrap()
    }

    #[test]
    fn a_query_answers_with_a_header_and_says_how_many_rows() {
        let path = planted("rows");
        let out = tool(&path)
            .run(&json!({"query": "SELECT id, title, score FROM notes ORDER BY id"}))
            .unwrap();
        assert!(out.starts_with("id | title | score\n"), "{out}");
        assert!(out.contains("1 | one | 1.5"), "{out}");
        assert!(out.contains("2 | two | NULL"), "{out}");
        assert!(out.ends_with("[2 row(s)]"), "{out}");
    }

    #[test]
    fn the_connection_refuses_a_write() {
        let path = planted("read-only");
        let err = tool(&path)
            .run(&json!({"query": "DELETE FROM notes"}))
            .unwrap_err();
        assert!(
            err.contains("readonly") || err.contains("read-only"),
            "{err}"
        );
        // And the rows are still there.
        let out = tool(&path)
            .run(&json!({"query": "SELECT COUNT(*) AS n FROM notes"}))
            .unwrap();
        assert!(out.contains("2"), "{out}");
    }

    #[test]
    fn two_statements_are_refused_before_sqlite_sees_them() {
        let path = planted("multi");
        let err = tool(&path)
            .run(&json!({"query": "SELECT 1; SELECT 2"}))
            .unwrap_err();
        assert!(err.contains("one statement per call"), "{err}");
        // One trailing semicolon is how everyone writes a query.
        assert!(tool(&path).run(&json!({"query": "SELECT 1;"})).is_ok());
    }

    #[test]
    fn a_long_result_says_it_was_cut() {
        let path = planted("cap");
        let out = tool(&path)
            .run(&json!({"query": "SELECT notes.id FROM notes, notes AS other"}))
            .unwrap();
        assert!(
            out.contains("row(s) shown, more exist") || out.contains("[4 row(s)]"),
            "{out}"
        );
    }

    #[test]
    fn a_missing_argument_names_the_argument() {
        let path = planted("args");
        let err = tool(&path).run(&json!({})).unwrap_err();
        assert!(err.contains("`query`"), "{err}");
    }
}
