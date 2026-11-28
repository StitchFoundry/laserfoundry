//! Update check, pure part: decide whether a release manifest announces a newer version, and verify
//! it. There is no network, file or UI code here; the caller fetches the manifest bytes, the detached
//! Ed25519 signature (hex) and, later, the download, and asks this crate what to believe.
//!
//! Rules: only a strictly newer `x.y.z` counts; anything unreadable is "not newer". The signature is
//! over the exact manifest bytes (Ed25519, RFC 8032, <https://www.rfc-editor.org/rfc/rfc8032>), and the
//! download is checked against a SHA-256 digest (FIPS 180-4).

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Largest manifest accepted; anything bigger is refused before parsing.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_NOTES_CHARS: usize = 4000;
const MAX_URL_CHARS: usize = 2048;
/// Largest number accepted in a version part, so comparisons never overflow.
const MAX_VERSION_PART: u64 = 999_999_999;

/// A release announced by a verified manifest that is newer than the running program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    pub notes: String,
    /// The page to open (https only). Nothing is downloaded or installed from here.
    pub url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateError {
    /// Manifest too large, not JSON, or missing a field.
    BadManifest,
    /// The signature is not valid for the manifest under the trusted key.
    BadSignature,
    /// The manifest is for another program.
    WrongProgram,
    /// The manifest version is the same, older, or unreadable.
    NotNewer,
}

/// Parse `x.y.z` (an optional leading `v` is allowed): three plain decimal parts, nothing else.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.trim();
    let text = text.strip_prefix('v').unwrap_or(text);
    let mut parts = text.split('.');
    let major = version_part(parts.next()?)?;
    let minor = version_part(parts.next()?)?;
    let patch = version_part(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// True only when `candidate` is strictly newer than `running`; unreadable either way is false.
pub fn is_newer(running: &str, candidate: &str) -> bool {
    match (parse_version(running), parse_version(candidate)) {
        (Some(running), Some(candidate)) => candidate > running,
        _ => false,
    }
}

/// Check a detached hex signature over `manifest` against the trusted 32-byte public key.
pub fn verify_signature(manifest: &[u8], signature_hex: &str, public_key: &[u8; 32]) -> bool {
    let Some(sig) = decode_hex(signature_hex.trim()) else { return false };
    let Ok(sig) = Signature::from_slice(&sig) else { return false };
    let Ok(key) = VerifyingKey::from_bytes(public_key) else { return false };
    key.verify(manifest, &sig).is_ok()
}

/// Check a download against its hex SHA-256 digest (case-insensitive).
pub fn verify_sha256(data: &[u8], digest_hex: &str) -> bool {
    let Some(want) = decode_hex(digest_hex.trim()) else { return false };
    want.len() == 32 && want.as_slice() == Sha256::digest(data).as_slice()
}

/// The whole decision: verify the manifest, then report an update only if it is for `program` and
/// strictly newer than `running`. The signature is checked before anything else is read.
pub fn check(program: &str, running: &str, manifest: &[u8], signature_hex: &str, public_key: &[u8; 32]) -> Result<Update, UpdateError> {
    if manifest.len() > MAX_MANIFEST_BYTES {
        return Err(UpdateError::BadManifest);
    }
    if !verify_signature(manifest, signature_hex, public_key) {
        return Err(UpdateError::BadSignature);
    }
    let json: Value = serde_json::from_slice(manifest).map_err(|_| UpdateError::BadManifest)?;
    let field = |name: &str| json.get(name).and_then(Value::as_str);
    if field("program").ok_or(UpdateError::BadManifest)? != program {
        return Err(UpdateError::WrongProgram);
    }
    let version = field("version").ok_or(UpdateError::BadManifest)?;
    if !is_newer(running, version) {
        return Err(UpdateError::NotNewer);
    }
    let url = field("url").ok_or(UpdateError::BadManifest)?;
    if !url.starts_with("https://") || url.chars().count() > MAX_URL_CHARS || url.chars().any(char::is_control) {
        return Err(UpdateError::BadManifest);
    }
    let notes: String = field("notes").unwrap_or("").chars().take(MAX_NOTES_CHARS).collect();
    Ok(Update { version: version.trim().trim_start_matches('v').to_string(), notes, url: url.to_string() })
}

fn version_part(text: &str) -> Option<u64> {
    if text.is_empty() || text.len() > 9 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok().filter(|n| *n <= MAX_VERSION_PART)
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len() / 2).map(|i| u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    mod versions {
        use super::*;

        #[test]
        fn only_strictly_newer_counts() {
            assert!(is_newer("0.6.0", "0.6.1"));
            assert!(is_newer("0.6.9", "0.10.0"));
            assert!(is_newer("1.2.3", "v2.0.0"));
            assert!(!is_newer("0.6.0", "0.6.0"));
            assert!(!is_newer("0.6.1", "0.6.0"), "a downgrade is not an update");
            assert!(!is_newer("1.0.0", "0.99.99"));
        }

        #[test]
        fn garbled_versions_are_not_newer() {
            for bad in ["", "1", "1.2", "1.2.3.4", "1.2.x", "1.2.3-beta", "-1.0.0", "1..3", "+1.2.3", "９.0.0", "99999999999.0.0", "1.2.3\0"] {
                assert!(!is_newer("0.0.1", bad), "{bad:?}");
            }
            assert!(!is_newer("garbage", "9.9.9"));
        }
    }

    mod verification {
        use super::*;

        #[test]
        fn newer_signed_manifest_is_an_update() {
            let (key, m) = (signing_key(), manifest("laserfoundry", "0.7.0"));
            let update = check("laserfoundry", "0.6.0", m.as_bytes(), &sign(&key, m.as_bytes()), &public(&key)).unwrap();
            assert_eq!(update.version, "0.7.0");
            assert_eq!(update.notes, "Fixes.");
            assert_eq!(update.url, "https://example.org/releases/0.7.0");
        }

        #[test]
        fn forged_or_foreign_signature_is_refused() {
            let (key, m) = (signing_key(), manifest("laserfoundry", "0.7.0"));
            let other = SigningKey::from_bytes(&[9; 32]);
            let pk = public(&key);
            assert_eq!(check("laserfoundry", "0.6.0", m.as_bytes(), &sign(&other, m.as_bytes()), &pk), Err(UpdateError::BadSignature));
            let tampered = m.replace("0.7.0", "9.9.9");
            assert_eq!(check("laserfoundry", "0.6.0", tampered.as_bytes(), &sign(&key, m.as_bytes()), &pk), Err(UpdateError::BadSignature));
            for bad in ["", "zz", "abc", &"0".repeat(128), &"é".repeat(64)] {
                assert_eq!(check("laserfoundry", "0.6.0", m.as_bytes(), bad, &pk), Err(UpdateError::BadSignature), "{bad:?}");
            }
            assert_eq!(check("laserfoundry", "0.6.0", m.as_bytes(), &sign(&key, m.as_bytes()), &[0; 32]), Err(UpdateError::BadSignature));
        }

        #[test]
        fn downgrade_same_version_and_other_program_show_nothing() {
            let key = signing_key();
            let pk = public(&key);
            let run = |program: &str, version: &str| {
                let m = manifest(program, version);
                check("laserfoundry", "0.6.0", m.as_bytes(), &sign(&key, m.as_bytes()), &pk)
            };
            assert_eq!(run("laserfoundry", "0.5.0"), Err(UpdateError::NotNewer));
            assert_eq!(run("laserfoundry", "0.6.0"), Err(UpdateError::NotNewer));
            assert_eq!(run("laserfoundry", "banana"), Err(UpdateError::NotNewer));
            assert_eq!(run("stitchbench", "0.7.0"), Err(UpdateError::WrongProgram));
        }

        #[test]
        fn hostile_manifests_are_refused() {
            let key = signing_key();
            let pk = public(&key);
            let signed = |m: &[u8]| check("laserfoundry", "0.6.0", m, &sign(&key, m), &pk);
            assert_eq!(signed(b"not json"), Err(UpdateError::BadManifest));
            assert_eq!(signed(b"[]"), Err(UpdateError::BadManifest));
            assert_eq!(signed(br#"{"program":"laserfoundry","version":"0.7.0"}"#), Err(UpdateError::BadManifest));
            let http = manifest("laserfoundry", "0.7.0").replace("https://", "http://");
            assert_eq!(signed(http.as_bytes()), Err(UpdateError::BadManifest));
            let js = manifest("laserfoundry", "0.7.0").replace("https://example.org/releases/0.7.0", "javascript:alert(1)");
            assert_eq!(signed(js.as_bytes()), Err(UpdateError::BadManifest));
            assert_eq!(signed(&vec![b' '; MAX_MANIFEST_BYTES + 1]), Err(UpdateError::BadManifest));
        }

        #[test]
        fn download_digest_is_checked() {
            let hex = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
            assert!(verify_sha256(b"abc", hex));
            assert!(verify_sha256(b"abc", &hex.to_uppercase()));
            assert!(!verify_sha256(b"abd", hex));
            assert!(!verify_sha256(b"abc", &hex[..62]));
            assert!(!verify_sha256(b"abc", ""));
            assert!(!verify_sha256(b"abc", "not hex"));
        }
    }

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7; 32])
    }

    fn public(key: &SigningKey) -> [u8; 32] {
        key.verifying_key().to_bytes()
    }

    fn sign(key: &SigningKey, data: &[u8]) -> String {
        key.sign(data).to_bytes().iter().map(|b| format!("{b:02x}")).collect()
    }

    fn manifest(program: &str, version: &str) -> String {
        format!(r#"{{"program":"{program}","version":"{version}","notes":"Fixes.","url":"https://example.org/releases/{version}"}}"#)
    }
}
