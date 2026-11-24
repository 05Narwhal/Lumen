//! Package signatures (spec §8): a blake3 digest over the file list, signed with minisign.
//!
//! `SIGNATURE` holds minisign signature text, or `tauri signer sign`'s base64 of it. A signature
//! by the built-in JL Studios key needs nothing else. An author signature carries its public key
//! as the last word of the untrusted comment: `untrusted comment: lumen plugin key <base64 key>`.
//! That line isn't signed, but the signature must verify under that key, so a swapped key fails.

use std::collections::BTreeMap;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use minisign_verify::{PublicKey, Signature};

use crate::package::Unpacked;
use crate::{PluginError, PluginTrust};

/// The prefix of an author signature's untrusted comment.
pub const AUTHOR_KEY_COMMENT: &str = "lumen plugin key";

/// blake3 over `path\0<blake3(file) hex>\n` for every file in byte order of the path
/// (`SIGNATURE` is never in `files`).
pub fn digest(files: &BTreeMap<String, [u8; 32]>) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    for (path, hash) in files {
        h.update(path.as_bytes());
        h.update(b"\0");
        h.update(hex(hash).as_bytes());
        h.update(b"\n");
    }
    *h.finalize().as_bytes()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The keys Lumen trusts: JL Studios' built-in key, when this build has one.
pub struct Keys {
    pub jl: Option<PublicKey>,
}

impl Keys {
    /// Accepts a raw minisign key line (`RW…`), the two-line minisign key text, or `tauri
    /// signer generate`'s base64 of that text.
    pub fn from_base64(jl: Option<&str>) -> Result<Keys, PluginError> {
        let jl = match jl.map(str::trim).filter(|s| !s.is_empty()) {
            None => None,
            Some(s) => Some(
                parse_public_key(s)
                    .ok_or_else(|| PluginError::signature("The built-in plugin key isn't valid"))?,
            ),
        };
        Ok(Keys { jl })
    }
}

fn parse_public_key(s: &str) -> Option<PublicKey> {
    let s = s.trim();
    if s.contains('\n') {
        return PublicKey::decode(s).ok();
    }
    if let Ok(k) = PublicKey::from_base64(s) {
        return Some(k);
    }
    let text = String::from_utf8(B64.decode(s).ok()?).ok()?;
    PublicKey::decode(text.trim()).ok()
}

/// The signature text, unwrapping `tauri signer`'s base64.
fn signature_text(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.starts_with("untrusted comment:") {
        return Some(raw.to_string());
    }
    let compact: String = raw.split_whitespace().collect();
    String::from_utf8(B64.decode(compact).ok()?).ok()
}

/// The 8-byte key id from the signature's second line (`ED` + key id + signature).
fn key_id(text: &str) -> Option<[u8; 8]> {
    let line = text.lines().nth(1)?;
    let bin = B64.decode(line.trim()).ok()?;
    bin.get(2..10)?.try_into().ok()
}

/// "A1B2-C3D4": the first four bytes of the key id in upper hex.
pub fn fingerprint(key_id: &[u8; 8]) -> String {
    let h = hex(&key_id[..4]).to_uppercase();
    format!("{}-{}", &h[..4], &h[4..])
}

/// Checks a package's signature. `Ok((Unsigned, None))` when there is none; the store decides
/// whether unsigned is allowed and whether the trust is enough for the id.
pub fn verify(pkg: &Unpacked, keys: &Keys) -> Result<(PluginTrust, Option<String>), PluginError> {
    let Some(raw) = pkg.signature.as_deref() else {
        return Ok((PluginTrust::Unsigned, None));
    };
    let bad = || PluginError::signature("The package's signature doesn't match its files");
    let text = signature_text(raw)
        .ok_or_else(|| PluginError::signature("SIGNATURE isn't a minisign signature"))?;
    let sig = Signature::decode(&text)
        .map_err(|_| PluginError::signature("SIGNATURE isn't a minisign signature"))?;
    let id = key_id(&text)
        .ok_or_else(|| PluginError::signature("SIGNATURE isn't a minisign signature"))?;
    let id_hex = Some(hex(&id));
    let msg = digest(&pkg.files);

    if keys
        .jl
        .as_ref()
        .is_some_and(|jl| jl.verify(&msg, &sig, false).is_ok())
    {
        return Ok((PluginTrust::JlStudios, id_hex));
    }
    let author_key = sig
        .untrusted_comment()
        .strip_prefix("untrusted comment:")
        .unwrap_or(sig.untrusted_comment())
        .trim()
        .strip_prefix(AUTHOR_KEY_COMMENT)
        .and_then(|rest| rest.split_whitespace().last())
        .and_then(parse_public_key);
    let Some(key) = author_key else {
        // No key travels with it: only the JL key could have signed it, and it didn't verify.
        return Err(bad());
    };
    key.verify(&msg, &sig, false).map_err(|_| bad())?;
    let trust = PluginTrust::Author {
        name: pkg.manifest.author.clone(),
        fingerprint: fingerprint(&id),
    };
    Ok((trust, id_hex))
}

/// Signing for tests (this crate's and the store's).
#[cfg(test)]
pub(crate) mod fixture {
    use std::collections::BTreeMap;
    use std::io::Cursor;

    use super::*;

    pub struct TestKey {
        pub pk: minisign::PublicKey,
        pub sk: minisign::SecretKey,
    }

    impl TestKey {
        pub fn generate() -> TestKey {
            let kp = minisign::KeyPair::generate_unencrypted_keypair().unwrap();
            TestKey {
                pk: kp.pk,
                sk: kp.sk,
            }
        }

        /// The checked-in key under `tests/fixtures/test-key/` (TEST ONLY).
        pub fn fixture() -> TestKey {
            let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test-key");
            let pk = std::fs::read_to_string(format!("{dir}/test.pub")).unwrap();
            let sk = std::fs::read_to_string(format!("{dir}/test.key")).unwrap();
            TestKey {
                pk: minisign::PublicKeyBox::from_string(&pk)
                    .unwrap()
                    .into_public_key()
                    .unwrap(),
                // `into_secret_key` prompts on stdin for None and fails 0.7's checksum for an
                // unencrypted key, so read the box's key line directly.
                sk: minisign::SecretKey::from_bytes(
                    &B64.decode(sk.lines().nth(1).unwrap().trim()).unwrap(),
                )
                .unwrap(),
            }
        }

        /// The raw `RW…` public key line.
        pub fn pub_b64(&self) -> String {
            self.pk.to_base64()
        }

        /// Minisign signature text over `msg` with `untrusted` as the untrusted comment.
        pub fn sign_text(&self, msg: &[u8], untrusted: &str) -> String {
            minisign::sign(
                Some(&self.pk),
                &self.sk,
                Cursor::new(msg),
                Some("timestamp:0"),
                Some(untrusted),
            )
            .unwrap()
            .into_string()
        }

        /// An author `SIGNATURE` for these package entries (the key travels in the comment).
        pub fn sign_entries(&self, entries: &[(&str, &[u8])]) -> String {
            let d = digest(&files_of(entries));
            self.sign_text(&d, &format!("{AUTHOR_KEY_COMMENT} {}", self.pub_b64()))
        }

        /// A `SIGNATURE` as the JL key writes it (`tauri signer sign`: base64, no key in it).
        pub fn sign_entries_as_jl(&self, entries: &[(&str, &[u8])]) -> String {
            let d = digest(&files_of(entries));
            B64.encode(self.sign_text(&d, "signature from tauri secret key"))
        }
    }

    pub fn files_of(entries: &[(&str, &[u8])]) -> BTreeMap<String, [u8; 32]> {
        entries
            .iter()
            .filter(|(p, _)| *p != "SIGNATURE")
            .map(|(p, d)| (p.to_string(), *blake3::hash(d).as_bytes()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;
    use crate::package::fixture::{manifest, pack};
    use crate::package::open_package;

    fn theme_entries() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            (
                "plugin.toml",
                manifest("[[prompts]]\nname = \"a\"\ndescription = \"d\"\nfile = \"themes/a.toml\"").into_bytes(),
            ),
            ("themes/a.toml", b"x = 1".to_vec()),
        ]
    }

    fn open_signed(
        entries: &[(&str, Vec<u8>)],
        sig: Option<String>,
    ) -> (tempfile::TempDir, Unpacked) {
        let mut all: Vec<(&str, &[u8])> = entries.iter().map(|(p, d)| (*p, d.as_slice())).collect();
        if let Some(s) = &sig {
            all.push(("SIGNATURE", s.as_bytes()));
        }
        let (dir, path) = pack(&all);
        let pkg = open_package(&path, &dir.path().join("out")).unwrap();
        (dir, pkg)
    }

    fn borrowed<'a>(entries: &'a [(&'static str, Vec<u8>)]) -> Vec<(&'static str, &'a [u8])> {
        entries.iter().map(|(p, d)| (*p, d.as_slice())).collect()
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct GoldenFile {
        path: String,
        content_base64: String,
    }

    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Golden {
        note: String,
        files: Vec<GoldenFile>,
        digest_hex: String,
    }

    #[test]
    fn digest_golden() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/digest-golden.json"
        );
        let regen = std::env::var("LUMEN_REGEN_GOLDEN").as_deref() == Ok("1");
        let golden: Golden = if regen {
            let files = [
                ("plugin.toml", "id = \"com.test.x\"\n"),
                ("a-b", "dash"),
                ("a.b", "dot"),
                ("a/b", "slash"),
                ("A", "upper"),
                ("assets/caf\u{e9}.txt", "accented"),
                ("themes/empty.toml", ""),
            ];
            Golden {
                note: "Generated by photos-plugins signature::tests::digest_golden with LUMEN_REGEN_GOLDEN=1; \
                       D1's pack.test.mjs reads it. Paths sort by UTF-8 bytes."
                    .into(),
                files: files
                    .iter()
                    .map(|(p, c)| GoldenFile { path: p.to_string(), content_base64: B64.encode(c) })
                    .collect(),
                digest_hex: String::new(),
            }
        } else {
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
        };
        let files: BTreeMap<String, [u8; 32]> = golden
            .files
            .iter()
            .map(|f| {
                (
                    f.path.clone(),
                    *blake3::hash(&B64.decode(&f.content_base64).unwrap()).as_bytes(),
                )
            })
            .collect();
        let got = hex(&digest(&files));
        if regen {
            let out = Golden {
                digest_hex: got,
                ..golden
            };
            std::fs::write(path, serde_json::to_string_pretty(&out).unwrap() + "\n").unwrap();
        } else {
            assert_eq!(got, golden.digest_hex);
        }
    }

    #[test]
    fn digest_ignores_zip_order() {
        let e = theme_entries();
        let mut rev = e.clone();
        rev.reverse();
        let (_a, p1) = open_signed(&e, None);
        let (_b, p2) = open_signed(&rev, None);
        assert_eq!(digest(&p1.files), digest(&p2.files));
        assert_ne!(digest(&p1.files), digest(&BTreeMap::new()));
    }

    #[test]
    fn author_signature_verifies() {
        let key = TestKey::fixture();
        let e = theme_entries();
        let (_d, pkg) = open_signed(&e, Some(key.sign_entries(&borrowed(&e))));
        let (trust, id) = verify(&pkg, &Keys::from_base64(None).unwrap()).unwrap();
        let kid: [u8; 8] = key.pk.keynum().try_into().unwrap();
        assert_eq!(
            trust,
            PluginTrust::Author {
                name: "Tester".into(),
                fingerprint: fingerprint(&kid)
            }
        );
        assert_eq!(id, Some(hex(&kid)));
    }

    #[test]
    fn tampered_file_fails() {
        let key = TestKey::fixture();
        let e = theme_entries();
        let sig = key.sign_entries(&borrowed(&e));
        let mut tampered = e.clone();
        tampered[1].1 = b"x = 2".to_vec();
        let (_d, pkg) = open_signed(&tampered, Some(sig));
        assert_eq!(
            verify(&pkg, &Keys { jl: None }).unwrap_err().code,
            "plugin_signature"
        );
    }

    #[test]
    fn swapped_key_fails() {
        let key = TestKey::fixture();
        let other = TestKey::generate();
        let e = theme_entries();
        let sig = key
            .sign_entries(&borrowed(&e))
            .replace(&key.pub_b64(), &other.pub_b64());
        let (_d, pkg) = open_signed(&e, Some(sig));
        assert_eq!(
            verify(&pkg, &Keys { jl: None }).unwrap_err().code,
            "plugin_signature"
        );
    }

    #[test]
    fn unsigned_is_unsigned() {
        let (_d, pkg) = open_signed(&theme_entries(), None);
        assert_eq!(
            verify(&pkg, &Keys { jl: None }).unwrap(),
            (PluginTrust::Unsigned, None)
        );
    }

    #[test]
    fn jl_key_gives_jl_trust() {
        let key = TestKey::fixture();
        let e = theme_entries();
        let (_d, pkg) = open_signed(&e, Some(key.sign_entries(&borrowed(&e))));
        let keys = Keys::from_base64(Some(&key.pub_b64())).unwrap();
        assert_eq!(verify(&pkg, &keys).unwrap().0, PluginTrust::JlStudios);
    }

    #[test]
    fn fingerprint_format() {
        assert_eq!(
            fingerprint(&[0xa1, 0xb2, 0xc3, 0xd4, 0, 0, 0, 0]),
            "A1B2-C3D4"
        );
    }

    #[test]
    fn tauri_signer_pubkey_accepted() {
        let key = TestKey::fixture();
        let two_line = key.pk.to_box().unwrap().into_string();
        for form in [key.pub_b64(), two_line.clone(), B64.encode(&two_line)] {
            assert!(
                Keys::from_base64(Some(&form)).unwrap().jl.is_some(),
                "{form}"
            );
        }
        assert!(Keys::from_base64(Some("")).unwrap().jl.is_none());
        assert_eq!(
            Keys::from_base64(Some("nonsense")).err().unwrap().code,
            "plugin_signature"
        );
    }

    #[test]
    fn tauri_signer_signature_accepted() {
        let key = TestKey::fixture();
        let e = theme_entries();
        let keys =
            Keys::from_base64(Some(&B64.encode(key.pk.to_box().unwrap().into_string()))).unwrap();
        let (_d, pkg) = open_signed(&e, Some(key.sign_entries_as_jl(&borrowed(&e))));
        assert_eq!(verify(&pkg, &keys).unwrap().0, PluginTrust::JlStudios);
        // Without the JL key built in, a tauri signature carries no key and is refused.
        assert_eq!(
            verify(&pkg, &Keys { jl: None }).unwrap_err().code,
            "plugin_signature"
        );
    }

    #[test]
    #[ignore = "set LUMEN_PLUGIN_VERIFY=<package> (D1 runs it on pack.mjs output)"]
    fn verifies_external_package_from_env() {
        let path = std::env::var("LUMEN_PLUGIN_VERIFY").expect("LUMEN_PLUGIN_VERIFY=<path>");
        let dir = tempfile::tempdir().unwrap();
        let pkg = open_package(std::path::Path::new(&path), &dir.path().join("out")).unwrap();
        let jl = std::env::var("LUMEN_PLUGIN_PUBKEY").ok();
        let (trust, id) = verify(&pkg, &Keys::from_base64(jl.as_deref()).unwrap()).unwrap();
        assert_ne!(trust, PluginTrust::Unsigned, "the package isn't signed");
        println!(
            "{} {}: {trust:?} key {id:?}",
            pkg.manifest.id, pkg.manifest.version
        );
    }

    #[test]
    #[ignore = "writes tests/fixtures/test-key; run once"]
    fn make_test_key() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test-key");
        std::fs::create_dir_all(dir).unwrap();
        let k = TestKey::generate();
        std::fs::write(
            format!("{dir}/test.pub"),
            k.pk.to_box().unwrap().into_string(),
        )
        .unwrap();
        std::fs::write(
            format!("{dir}/test.key"),
            k.sk.to_box(None).unwrap().into_string(),
        )
        .unwrap();
    }
}
