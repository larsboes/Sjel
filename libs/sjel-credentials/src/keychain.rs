use crate::{
    CredentialError, CredentialId, CredentialProvider, Presence, ProviderKind, ProviderReference,
    SecretValue,
};
use std::io::Write;
use std::process::{Command, Stdio};

/// Redacted result from a subprocess. Standard error is intentionally discarded by provider
/// errors because platform tools can include credential metadata in diagnostics.
#[derive(Clone, Debug)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
}

/// Injectable command boundary for deterministic provider tests.
pub trait CommandRunner: Send + Sync {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        stdin: Option<&[u8]>,
    ) -> Result<CommandOutput, CredentialError>;
}

impl<T: CommandRunner + ?Sized> CommandRunner for &T {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        stdin: Option<&[u8]>,
    ) -> Result<CommandOutput, CredentialError> {
        (*self).run(program, args, stdin)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        stdin: Option<&[u8]>,
    ) -> Result<CommandOutput, CredentialError> {
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (program, args, stdin);
            Err(CredentialError::UnsupportedPlatform)
        }

        #[cfg(target_os = "macos")]
        {
            let mut command = Command::new(program);
            command.args(args);
            if stdin.is_some() {
                command.stdin(Stdio::piped());
            }
            command.stdout(Stdio::piped()).stderr(Stdio::null());
            let mut child = command
                .spawn()
                .map_err(|_| CredentialError::ProviderOperation { operation: "start" })?;
            if let Some(input) = stdin {
                let mut child_stdin = child
                    .stdin
                    .take()
                    .ok_or(CredentialError::ProviderOperation { operation: "write" })?;
                child_stdin
                    .write_all(input)
                    .map_err(|_| CredentialError::ProviderOperation { operation: "write" })?;
            }
            let output = child
                .wait_with_output()
                .map_err(|_| CredentialError::ProviderOperation { operation: "wait" })?;
            Ok(CommandOutput {
                success: output.status.success(),
                stdout: output.stdout,
            })
        }
    }
}

/// macOS Keychain generic-password adapter.
///
/// Values are hex encoded into the `security -i` input stream. They do not appear in process
/// arguments or shell history. Reads return the value only to the Rust caller; errors never
/// include command output.
pub struct KeychainProvider<R = SystemCommandRunner> {
    runner: R,
}

impl KeychainProvider<SystemCommandRunner> {
    pub fn system() -> Self {
        Self {
            runner: SystemCommandRunner,
        }
    }
}

impl<R> KeychainProvider<R> {
    pub fn with_runner(runner: R) -> Self {
        Self { runner }
    }

    fn location(
        id: &CredentialId,
        reference: &ProviderReference,
    ) -> Result<(String, String), CredentialError> {
        let ProviderReference::Keychain { account } = reference else {
            return Err(CredentialError::WrongProviderReference);
        };
        validate_atom(account)?;
        Ok((
            format!("com.sjel.credential.{}", id.as_str()),
            account.clone(),
        ))
    }

    fn write_secret(
        &self,
        id: &CredentialId,
        reference: &ProviderReference,
        value: &SecretValue,
        replace: bool,
    ) -> Result<(), CredentialError>
    where
        R: CommandRunner,
    {
        let (service, account) = Self::location(id, reference)?;
        let encoded = zeroize::Zeroizing::new(encode_hex(value.expose_bytes()));
        let replace_flag = if replace { "-U " } else { "" };
        let command = zeroize::Zeroizing::new(format!(
            "add-generic-password {replace_flag}-s {service} -a {account} -w {}\n",
            encoded.as_str()
        ));
        let output = self
            .runner
            .run("security", &["-i"], Some(command.as_bytes()))?;
        if output.success {
            Ok(())
        } else {
            Err(CredentialError::ProviderOperation { operation: "write" })
        }
    }
}

impl<R: CommandRunner> CredentialProvider for KeychainProvider<R> {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Keychain
    }

    fn status(
        &self,
        id: &CredentialId,
        reference: &ProviderReference,
    ) -> Result<Presence, CredentialError> {
        let (service, account) = Self::location(id, reference)?;
        let output = self.runner.run(
            "security",
            &["find-generic-password", "-s", &service, "-a", &account],
            None,
        )?;
        Ok(if output.success {
            Presence::Present
        } else {
            Presence::Missing
        })
    }

    fn create(
        &self,
        id: &CredentialId,
        _label: &str,
        value: &SecretValue,
    ) -> Result<ProviderReference, CredentialError> {
        let reference = ProviderReference::Keychain {
            account: id.as_str().to_string(),
        };
        self.write_secret(id, &reference, value, false)?;
        Ok(reference)
    }

    fn update(
        &self,
        id: &CredentialId,
        reference: &ProviderReference,
        value: &SecretValue,
    ) -> Result<(), CredentialError> {
        self.write_secret(id, reference, value, true)
    }

    fn get(
        &self,
        id: &CredentialId,
        reference: &ProviderReference,
    ) -> Result<SecretValue, CredentialError> {
        let (service, account) = Self::location(id, reference)?;
        let output = self.runner.run(
            "security",
            &[
                "find-generic-password",
                "-w",
                "-s",
                &service,
                "-a",
                &account,
            ],
            None,
        )?;
        if !output.success {
            return Err(CredentialError::NotFound);
        }
        let encoded_output = zeroize::Zeroizing::new(output.stdout);
        let encoded = std::str::from_utf8(&encoded_output)
            .map_err(|_| CredentialError::InvalidSecretRepresentation)?
            .trim();
        let bytes = decode_hex(encoded)?;
        SecretValue::from_zeroizing(bytes)
    }

    fn delete(
        &self,
        id: &CredentialId,
        reference: &ProviderReference,
    ) -> Result<(), CredentialError> {
        let (service, account) = Self::location(id, reference)?;
        let output = self.runner.run(
            "security",
            &["delete-generic-password", "-s", &service, "-a", &account],
            None,
        )?;
        if output.success {
            Ok(())
        } else {
            Err(CredentialError::NotFound)
        }
    }
}

fn validate_atom(value: &str) -> Result<(), CredentialError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(CredentialError::InvalidReference);
    }
    Ok(())
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_hex(value: &str) -> Result<zeroize::Zeroizing<Vec<u8>>, CredentialError> {
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return Err(CredentialError::InvalidSecretRepresentation);
    }
    let (pairs, _) = value.as_bytes().as_chunks::<2>();
    let decoded: Vec<u8> = pairs
        .iter()
        .map(|pair| {
            let high = hex_nibble(pair[0]).ok_or(CredentialError::InvalidSecretRepresentation)?;
            let low = hex_nibble(pair[1]).ok_or(CredentialError::InvalidSecretRepresentation)?;
            Ok((high << 4) | low)
        })
        .collect::<Result<_, _>>()?;
    Ok(zeroize::Zeroizing::new(decoded))
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    type RecordedCall = (String, Vec<String>, Option<Vec<u8>>);

    #[derive(Default)]
    struct FakeRunner {
        calls: Mutex<Vec<RecordedCall>>,
        response: Mutex<Option<CommandOutput>>,
    }

    impl CommandRunner for FakeRunner {
        fn run(
            &self,
            program: &str,
            args: &[&str],
            stdin: Option<&[u8]>,
        ) -> Result<CommandOutput, CredentialError> {
            self.calls.lock().unwrap().push((
                program.into(),
                args.iter().map(|arg| (*arg).into()).collect(),
                stdin.map(ToOwned::to_owned),
            ));
            Ok(self
                .response
                .lock()
                .unwrap()
                .take()
                .unwrap_or(CommandOutput {
                    success: true,
                    stdout: Vec::new(),
                }))
        }
    }

    fn fixture() -> (CredentialId, ProviderReference) {
        (
            CredentialId::parse("inbound-auth").unwrap(),
            ProviderReference::Keychain {
                account: "sparpreis-watch".into(),
            },
        )
    }

    #[test]
    fn write_keeps_secret_out_of_process_arguments_and_uses_safe_input_encoding() {
        let runner = FakeRunner::default();
        let provider = KeychainProvider::with_runner(&runner);
        let (id, _) = fixture();
        let secret = SecretValue::new(b"sensitive-value\nwith-special-'chars".to_vec()).unwrap();

        let reference = provider.create(&id, "Inbound auth", &secret).unwrap();
        assert_eq!(
            reference,
            ProviderReference::Keychain {
                account: "inbound-auth".into()
            }
        );

        let calls = runner.calls.lock().unwrap();
        let (program, args, stdin) = &calls[0];
        assert_eq!(program, "security");
        assert_eq!(args, &["-i"]);
        let script = String::from_utf8(stdin.clone().unwrap()).unwrap();
        assert!(script.contains("add-generic-password -s"));
        assert!(!script.contains("add-generic-password -U"));
        assert!(script
            .contains("73656e7369746976652d76616c75650a776974682d7370656369616c2d276368617273"));
        assert!(!args.join(" ").contains("sensitive-value"));
    }

    #[test]
    fn update_uses_keychain_replace_semantics_without_secret_arguments() {
        let runner = FakeRunner::default();
        let provider = KeychainProvider::with_runner(&runner);
        let (id, reference) = fixture();
        let secret = SecretValue::new(b"replacement-value".to_vec()).unwrap();

        provider.update(&id, &reference, &secret).unwrap();

        let calls = runner.calls.lock().unwrap();
        let (program, args, stdin) = &calls[0];
        assert_eq!(program, "security");
        assert_eq!(args, &["-i"]);
        assert!(String::from_utf8(stdin.clone().unwrap())
            .unwrap()
            .contains("add-generic-password -U"));
        assert!(!args.join(" ").contains("replacement-value"));
    }

    #[test]
    fn reads_decode_hex_into_a_zeroizing_secret_value() {
        let runner = FakeRunner::default();
        *runner.response.lock().unwrap() = Some(CommandOutput {
            success: true,
            stdout: b"746f6b656e\n".to_vec(),
        });
        let provider = KeychainProvider::with_runner(&runner);
        let (id, reference) = fixture();

        let secret = provider.get(&id, &reference).unwrap();

        assert_eq!(secret.expose_bytes(), b"token");
    }

    #[test]
    fn malformed_references_and_wrong_provider_are_rejected_before_launch() {
        let runner = FakeRunner::default();
        let provider = KeychainProvider::with_runner(&runner);
        let (id, _) = fixture();
        let bad_account = ProviderReference::Keychain {
            account: "../../bad".into(),
        };
        assert_eq!(
            provider.status(&id, &bad_account),
            Err(CredentialError::InvalidReference)
        );
        let wrong = ProviderReference::Bitwarden {
            item_id: "item-id".into(),
        };
        assert_eq!(
            provider.status(&id, &wrong),
            Err(CredentialError::WrongProviderReference)
        );
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn hex_decoder_rejects_invalid_data() {
        assert_eq!(
            decode_hex(""),
            Err(CredentialError::InvalidSecretRepresentation)
        );
        assert_eq!(
            decode_hex("xyz"),
            Err(CredentialError::InvalidSecretRepresentation)
        );
        assert_eq!(
            decode_hex("0g"),
            Err(CredentialError::InvalidSecretRepresentation)
        );
        assert_eq!(decode_hex("4A").unwrap().as_slice(), &[0x4a]);
    }
}
