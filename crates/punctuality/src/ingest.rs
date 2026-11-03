//! Reading the monthly parquet and folding it into cells.
//!
//! Seven columns out of the upstream schema are projected, which is most of why a laptop can chew
//! through ~120M stop records without a database: parquet is columnar, so the fourteen
//! unread columns are never decompressed.

use crate::stats::Cell;
use arrow::array::{Array, BooleanArray, Int32Array, StringArray, TimestampNanosecondArray};
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ProjectionMask;
use std::collections::HashMap;
use std::path::Path;

const COLUMNS: [&str; 7] = [
    "eva",
    "train_type",
    "delay_in_min",
    "is_canceled",
    "arrival_is_canceled",
    "departure_is_canceled",
    "time",
];

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("opening {0}: {1}")]
    Open(String, #[source] std::io::Error),
    #[error("reading parquet {0}: {1}")]
    Parquet(String, #[source] parquet::errors::ParquetError),
    #[error("reading parquet {0}: {1}")]
    Arrow(String, #[source] arrow::error::ArrowError),
    #[error("{file}: column `{column}` is missing — upstream schema changed (see upstreams.toml [deutsche-bahn-data])")]
    MissingColumn { file: String, column: String },
    #[error("{file}: column `{column}` has type {actual}, expected {expected}")]
    WrongType {
        file: String,
        column: String,
        actual: String,
        expected: String,
    },
}

/// A station/type/hour/weekend bucket.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CellKey {
    pub eva: String,
    pub train_type: String,
    pub hour: u8,
    pub weekend: bool,
}

/// What one ingest run produced, so the caller can report it rather than guess.
#[derive(Debug, Default, Clone, Copy)]
pub struct Counts {
    pub rows: u64,
    /// Rows dropped for having no usable delay reading and no cancellation flag.
    pub skipped: u64,
}

/// Local wall-clock hour and weekend flag from a nanosecond timestamp.
///
/// No timezone library, deliberately: upstream states every timestamp is already
/// Europe/Berlin local time as the DB API returned it, with no conversion applied. So
/// the value IS the local clock, and treating it as one is correct — converting it
/// again would shift every reading by an hour or two and quietly move the rush hour.
fn hour_and_weekend(nanos: i64) -> (u8, bool) {
    let secs = nanos.div_euclid(1_000_000_000);
    let hour = (secs.rem_euclid(86_400) / 3_600) as u8;
    // Epoch day 0 was a Thursday; +3 rotates that to a Monday-first week.
    let weekday = (secs.div_euclid(86_400) + 3).rem_euclid(7);
    (hour, weekday >= 5)
}

fn column<'a, T: 'static>(
    batch: &'a RecordBatch,
    file: &str,
    name: &str,
    expected: &str,
) -> Result<&'a T, IngestError> {
    optional_column(batch, file, name, expected)?.ok_or_else(|| IngestError::MissingColumn {
        file: file.into(),
        column: name.into(),
    })
}

fn optional_column<'a, T: 'static>(
    batch: &'a RecordBatch,
    file: &str,
    name: &str,
    expected: &str,
) -> Result<Option<&'a T>, IngestError> {
    let Ok(idx) = batch.schema().index_of(name) else {
        return Ok(None);
    };
    let col = batch.column(idx);
    col.as_any()
        .downcast_ref::<T>()
        .map(Some)
        .ok_or_else(|| IngestError::WrongType {
            file: file.into(),
            column: name.into(),
            actual: format!("{:?}", col.data_type()),
            expected: expected.into(),
        })
}

/// Folds one monthly file into `cells`, adding to whatever is already there.
pub fn fold_file(
    path: &Path,
    cells: &mut HashMap<CellKey, Cell>,
    stations: &mut HashMap<String, String>,
) -> Result<Counts, IngestError> {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let file = std::fs::File::open(path).map_err(|e| IngestError::Open(name.clone(), e))?;

    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| IngestError::Parquet(name.clone(), e))?;

    // station_name rides along so the CLI can resolve a name to an eva without a second
    // pass or a hardcoded station list.
    let mut wanted: Vec<&str> = COLUMNS.to_vec();
    wanted.push("station_name");
    let schema = builder.parquet_schema();
    let indices: Vec<usize> = (0..schema.num_columns())
        .filter(|i| wanted.contains(&schema.column(*i).name()))
        .collect();
    let mask = ProjectionMask::roots(schema, indices);

    let reader = builder
        .with_projection(mask)
        .with_batch_size(65_536)
        .build()
        .map_err(|e| IngestError::Parquet(name.clone(), e))?;

    let mut counts = Counts::default();
    for batch in reader {
        let batch = batch.map_err(|e| IngestError::Arrow(name.clone(), e))?;
        let eva = column::<StringArray>(&batch, &name, "eva", "Utf8")?;
        let train_type = column::<StringArray>(&batch, &name, "train_type", "Utf8")?;
        let delay = column::<Int32Array>(&batch, &name, "delay_in_min", "Int32")?;
        let canceled = optional_column::<BooleanArray>(&batch, &name, "is_canceled", "Boolean")?;
        let arrival_canceled =
            optional_column::<BooleanArray>(&batch, &name, "arrival_is_canceled", "Boolean")?;
        let departure_canceled =
            optional_column::<BooleanArray>(&batch, &name, "departure_is_canceled", "Boolean")?;
        if canceled.is_none() && arrival_canceled.is_none() && departure_canceled.is_none() {
            let column = if batch.schema().index_of("arrival_is_canceled").is_err()
                && batch.schema().index_of("departure_is_canceled").is_err()
            {
                "is_canceled"
            } else {
                "is_canceled or arrival_is_canceled/departure_is_canceled"
            };
            return Err(IngestError::MissingColumn {
                file: name.clone(),
                column: column.into(),
            });
        }
        let time =
            column::<TimestampNanosecondArray>(&batch, &name, "time", "Timestamp(Nanosecond)")?;
        let station_name = column::<StringArray>(&batch, &name, "station_name", "Utf8")?;

        for row in 0..batch.num_rows() {
            counts.rows += 1;
            if eva.is_null(row) || train_type.is_null(row) || time.is_null(row) {
                counts.skipped += 1;
                continue;
            }
            let is_canceled = [canceled, arrival_canceled, departure_canceled]
                .into_iter()
                .flatten()
                .any(|column| !column.is_null(row) && column.value(row));
            if delay.is_null(row) && !is_canceled {
                // Neither a delay reading nor a cancellation: nothing this row can say.
                counts.skipped += 1;
                continue;
            }

            let (hour, weekend) = hour_and_weekend(time.value(row));
            let key = CellKey {
                eva: eva.value(row).to_string(),
                train_type: train_type.value(row).to_string(),
                hour,
                weekend,
            };
            cells.entry(key).or_default().record(
                if delay.is_null(row) {
                    0
                } else {
                    delay.value(row)
                },
                is_canceled,
            );

            if !station_name.is_null(row) {
                stations
                    .entry(eva.value(row).to_string())
                    .or_insert_with(|| station_name.value(row).to_string());
            }
        }
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
    use parquet::arrow::ArrowWriter;
    use std::sync::Arc;

    const DAY: i64 = 86_400 * 1_000_000_000;
    const HOUR: i64 = 3_600 * 1_000_000_000;

    /// Writes a miniature parquet file with `parquet`'s own `ArrowWriter`.
    ///
    /// Generated rather than committed, on purpose. The real monthly publications are
    /// private and 100+ MB, so they cannot be the fixture; a checked-in blob could be,
    /// but it would freeze a byte layout that nobody could regenerate once the crate
    /// moved. Writing it with the same stack the reader reads it with is what makes
    /// this test move WITH an arrow bump instead of merely surviving one -- which is
    /// the whole reason it exists: verifying 59.1.0 -> 59.2.0 took a throwaway harness
    /// against a private file, and that evidence was thrown away with it.
    ///
    /// `omit` drops one column from the file, which is how the upstream-schema-change
    /// path gets a red to prove itself against. `upstreams.toml [deutsche-bahn-data]`
    /// records the 2026-05 and 2026-08 schema changes.
    fn write_fixture(path: &Path, omit: Option<&str>) {
        let columns: Vec<(&str, DataType)> = vec![
            ("eva", DataType::Utf8),
            ("train_type", DataType::Utf8),
            ("delay_in_min", DataType::Int32),
            ("is_canceled", DataType::Boolean),
            ("time", DataType::Timestamp(TimeUnit::Nanosecond, None)),
            ("station_name", DataType::Utf8),
        ];

        // Six rows, one per behaviour the reader has to get right. Saturday 09:00 and
        // Monday 17:00 are `2 * DAY + 9 * HOUR` and `4 * DAY + 17 * HOUR` from the
        // epoch, so the expected cell key is arithmetic rather than a lookup.
        let eva = StringArray::from(vec![
            Some("8000207"),
            Some("8000207"),
            Some("8000207"),
            Some("8000207"),
            None,
            Some("8000105"),
        ]);
        let train_type = StringArray::from(vec![
            Some("RE"),
            Some("RE"),
            Some("RE"),
            Some("RE"),
            Some("RE"),
            Some("ICE"),
        ]);
        let delay = Int32Array::from(vec![Some(3), Some(11), None, None, Some(1), Some(0)]);
        let canceled = BooleanArray::from(vec![
            Some(false),
            Some(false),
            Some(true),
            Some(false),
            Some(false),
            Some(false),
        ]);
        let time = TimestampNanosecondArray::from(vec![
            2 * DAY + 9 * HOUR,
            2 * DAY + 9 * HOUR,
            2 * DAY + 9 * HOUR,
            2 * DAY + 9 * HOUR,
            2 * DAY + 9 * HOUR,
            4 * DAY + 17 * HOUR,
        ]);
        let station_name = StringArray::from(vec![
            Some("Köln Hbf"),
            Some("Köln Hbf"),
            Some("Köln Hbf"),
            Some("Köln Hbf"),
            Some("Köln Hbf"),
            Some("Frankfurt(Main)Hbf"),
        ]);
        let arrays: Vec<(&str, arrow::array::ArrayRef)> = vec![
            ("eva", Arc::new(eva)),
            ("train_type", Arc::new(train_type)),
            ("delay_in_min", Arc::new(delay)),
            ("is_canceled", Arc::new(canceled)),
            ("time", Arc::new(time)),
            ("station_name", Arc::new(station_name)),
        ];

        let fields: Vec<Field> = columns
            .iter()
            .filter(|(name, _)| Some(*name) != omit)
            .map(|(name, ty)| Field::new(*name, ty.clone(), true))
            .collect();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(fields)),
            arrays
                .into_iter()
                .filter(|(name, _)| Some(*name) != omit)
                .map(|(_, a)| a)
                .collect(),
        )
        .expect("fixture columns are the same length");

        let file = std::fs::File::create(path).expect("fixture path is writable");
        let mut writer =
            ArrowWriter::try_new(file, batch.schema(), None).expect("writer accepts the schema");
        writer.write(&batch).expect("one batch writes");
        writer.close().expect("footer writes");
    }

    /// The system scratch dir, which is neither inside the repo nor inside the private
    /// data dir — the property this helper exists for. It read `TEST_TMPDIR` first until
    /// 2026-08-25 (PRD Q44), because Bazel handed every test its own; `cargo test` sets
    /// no such variable, so that branch could only ever be dead here.
    fn fixture_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(name)
    }

    /// The reader's whole job, on a file: project five columns, fold the rows that say
    /// something, drop the ones that do not, and key each cell on the local clock.
    #[test]
    fn folding_a_file_lands_every_row_in_the_bucket_it_belongs_to() {
        let path = fixture_path("fold-complete.parquet");
        write_fixture(&path, None);

        let mut cells = HashMap::new();
        let mut stations = HashMap::new();
        let counts = fold_file(&path, &mut cells, &mut stations).expect("the fixture reads");

        // Two rows say nothing: one with neither a delay reading nor a cancellation,
        // and one with a null eva. Both are skipped rather than folded as a zero.
        assert_eq!(counts.rows, 6);
        assert_eq!(counts.skipped, 2);

        // Saturday 09:00, from a timestamp that came out of the file rather than from
        // a synthetic integer -- which is the gap this test was written to close.
        let koeln = CellKey {
            eva: "8000207".into(),
            train_type: "RE".into(),
            hour: 9,
            weekend: true,
        };
        let cell = cells.get(&koeln).expect("the Köln cell exists");
        // The cancellation is counted as one, and is NOT in `n`: a cancelled train has
        // no delay, and folding it in as "0 minutes late" would let the worst outcome
        // improve the statistic.
        assert_eq!(cell.n, 2, "two delay readings, 3 and 11");
        assert_eq!(cell.canceled, 1);

        // Monday 17:00, a different station and type, so the key really is all four
        // fields and not just the station.
        let frankfurt = CellKey {
            eva: "8000105".into(),
            train_type: "ICE".into(),
            hour: 17,
            weekend: false,
        };
        assert_eq!(cells.get(&frankfurt).expect("the ICE cell exists").n, 1);
        assert_eq!(cells.len(), 2);

        // station_name rides along so the CLI can resolve a name without a second pass.
        assert_eq!(
            stations.get("8000207").map(String::as_str),
            Some("Köln Hbf")
        );
        assert_eq!(
            stations.get("8000105").map(String::as_str),
            Some("Frankfurt(Main)Hbf")
        );
    }

    /// The path that already broke once upstream, proven red. Without this, a dropped
    /// column is discovered by a nightly ingest producing a smaller number than
    /// yesterday's, which is not a discovery anyone makes on time.
    #[test]
    fn a_dropped_column_is_named_rather_than_panicked_or_silently_zeroed() {
        for missing in ["is_canceled", "delay_in_min", "train_type", "eva", "time"] {
            let path = fixture_path(&format!("fold-without-{missing}.parquet"));
            write_fixture(&path, Some(missing));

            let mut cells = HashMap::new();
            let mut stations = HashMap::new();
            match fold_file(&path, &mut cells, &mut stations) {
                Err(IngestError::MissingColumn { column, .. }) => {
                    assert_eq!(column, missing)
                }
                other => panic!("dropping `{missing}` should name it, got {other:?}"),
            }
            // Nothing was folded from a file the reader could not understand.
            assert!(cells.is_empty(), "no cells written for a broken schema");
        }

        // Green on the same code path, so the red above is the column and not the
        // fixture: the complete file still folds.
        let path = fixture_path("fold-complete-again.parquet");
        write_fixture(&path, None);
        let mut cells = HashMap::new();
        let mut stations = HashMap::new();
        assert!(fold_file(&path, &mut cells, &mut stations).is_ok());
        assert_eq!(cells.len(), 2);
    }

    #[test]
    fn epoch_zero_is_thursday_midnight() {
        let (hour, weekend) = hour_and_weekend(0);
        assert_eq!(hour, 0);
        assert!(!weekend, "1970-01-01 was a Thursday");
    }

    #[test]
    fn the_weekend_is_saturday_and_sunday() {
        // Thursday +2 = Saturday, +3 = Sunday, +4 = Monday.
        assert!(!hour_and_weekend(DAY).1, "Friday");
        assert!(hour_and_weekend(2 * DAY).1, "Saturday");
        assert!(hour_and_weekend(3 * DAY).1, "Sunday");
        assert!(!hour_and_weekend(4 * DAY).1, "Monday");
    }

    #[test]
    fn hours_advance_and_wrap_within_the_day() {
        assert_eq!(hour_and_weekend(9 * HOUR).0, 9);
        assert_eq!(hour_and_weekend(23 * HOUR).0, 23);
        assert_eq!(hour_and_weekend(24 * HOUR).0, 0);
    }

    #[test]
    fn timestamps_before_the_epoch_do_not_wrap_backwards() {
        // div_euclid rather than plain division: truncating toward zero would put
        // 1969-12-31 23:00 in hour -1, and a negative index into a 24-hour bucket is a
        // panic waiting for the one row with a bad timestamp.
        let (hour, _) = hour_and_weekend(-HOUR);
        assert_eq!(hour, 23);
        let (hour, weekend) = hour_and_weekend(-DAY);
        assert_eq!(hour, 0);
        assert!(!weekend, "1969-12-31 was a Wednesday");
    }
}
