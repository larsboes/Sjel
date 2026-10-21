//! Mint a primary's JWT key pair, and a token signed with it, for a **scratch** primary.
//!
//! `sjel setup` is the real path — it keeps the private key in the OS keychain and hands a token
//! to a tool's stdin, so the value never reaches a log or an argument vector. This example exists
//! for the case that has no keychain yet: proving the auth path against a throwaway `sqld` on
//! loopback, and reminting when one expires.
//!
//! ```console
//! # first run: generate, and write the public half where sqld reads it
//! $ cargo run -p sjel-turso --example primary-key -- --new --public-out /tmp/primary.pub
//! token: eyJhbGciOiJFZERTQSIs...
//!
//! # later: same key, fresh token
//! $ cargo run -p sjel-turso --example primary-key -- --key /tmp/primary.pkcs8
//! ```
//!
//! The token goes to stdout because a scratch primary's token protects nothing. A real one is
//! written by `sjel setup`, and its private key never leaves the keychain.
#![cfg(feature = "replica")]

use std::path::PathBuf;

use sjel_turso::{Access, PrimaryKey};

fn main() -> Result<(), sjel_turso::Error> {
    let mut key_path: Option<PathBuf> = None;
    let mut key_out: Option<PathBuf> = None;
    let mut public_out: Option<PathBuf> = None;
    let mut days: u64 = 30;
    let mut read_only = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--new" => {}
            "--key" => {
                key_path = Some(PathBuf::from(
                    args.next()
                        .ok_or("--key needs a path to the PKCS#8 document")?,
                ));
            }
            "--key-out" => {
                key_out = Some(PathBuf::from(args.next().ok_or("--key-out needs a path")?));
            }
            "--public-out" => {
                public_out = Some(PathBuf::from(
                    args.next().ok_or("--public-out needs a path")?,
                ));
            }
            "--days" => {
                days = args
                    .next()
                    .ok_or("--days needs a number")?
                    .parse()
                    .map_err(|_| "--days needs a whole number of days")?;
            }
            "--read-only" => read_only = true,
            other => return Err(format!("unknown argument {other:?}").into()),
        }
    }

    let key = match key_path {
        Some(path) => {
            let bytes = std::fs::read(&path)
                .map_err(|e| format!("could not read {}: {e}", path.display()))?;
            PrimaryKey::from_pkcs8(&bytes)?
        }
        None => PrimaryKey::generate()?,
    };

    if let Some(path) = key_out {
        std::fs::write(&path, key.pkcs8())
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
        eprintln!(
            "private key: {} — a plain file, for a scratch primary only",
            path.display()
        );
    }

    if let Some(path) = public_out {
        std::fs::write(&path, key.public_key_base64url())
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
        eprintln!("public key: {}", path.display());
    } else {
        eprintln!("public key: {}", key.public_key_base64url());
    }

    let access = if read_only {
        Access::Read
    } else {
        Access::Write
    };
    println!("{}", key.mint_for_days(None, access, days)?);
    Ok(())
}
