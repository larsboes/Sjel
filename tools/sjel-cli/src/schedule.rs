//! A service.toml `schedule` as whole seconds: `<N>m`, `<N>h` or `<N>d`.
//!
//! The runner's copy of tools/lib/schedule.sh, which tools/check-service-tomls.sh still reads.
//! Two copies of one rule, so both are tested against tools/lib/schedule-cases.tsv: a gate that
//! accepts a spec the runner then refuses, or the reverse, fails a test instead of drifting.

/// Seconds, or the reason the spec is refused, worded as the shell parser words it.
pub fn seconds(spec: &str) -> Result<u64, String> {
    let (n, unit) = match spec.char_indices().last() {
        Some((i, c @ ('m' | 'h' | 'd'))) => (&spec[..i], c),
        _ => {
            return Err(format!(
                "schedule = \"{spec}\" — expected <N>m, <N>h or <N>d (minutes, hours, days)"
            ))
        }
    };
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!(
            "schedule = \"{spec}\" — '{n}' is not a whole number"
        ));
    }
    let n: u64 = n
        .parse()
        .map_err(|_| format!("schedule = \"{spec}\" — '{n}' is not a whole number"))?;
    if n == 0 {
        return Err(format!("schedule = \"{spec}\" — must be greater than zero"));
    }
    let per = match unit {
        'm' => 60,
        'h' => 3600,
        _ => 86400,
    };
    n.checked_mul(per)
        .ok_or_else(|| format!("schedule = \"{spec}\" — '{n}' is not a whole number"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn agrees_with_the_shared_case_table() {
        let table = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../lib/schedule-cases.tsv"
        ))
        .unwrap();
        let mut n = 0;
        for line in table
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            let (spec, want) = line.split_once('\t').unwrap();
            let got = super::seconds(spec);
            match want.strip_prefix("error:") {
                Some(text) => assert!(
                    got.as_ref().is_err_and(|e| e.contains(text)),
                    "{spec}: {got:?}"
                ),
                None => assert_eq!(got, Ok(want.parse().unwrap()), "{spec}"),
            }
            n += 1;
        }
        assert!(n >= 10, "only {n} cases read");
    }
}
