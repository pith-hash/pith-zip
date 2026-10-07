//! The fixture harness: every committed archive under `tests/fixtures/`
//! is parsed and extracted by `pith-zip` and checked member-by-member
//! against the corpus in `tests/fixtures.rs` — the same fixed point
//! `gen-reference` pins into `reference.json`, asserted from the test
//! side so the reference can never drift from the suite silently.
//!
//! Malformed-archive behaviour is *not* re-tested here: the ported
//! `tests/zip.rs` suite owns every refusal path byte-for-byte. This file
//! only pins the valid fixtures the reference vectors are cut from.

#[path = "fixtures.rs"]
mod fixtures;

use fixtures::FIXTURES;
use pith_zip::ZipArchive;

/// Parses `file` out of `tests/fixtures/`. A fixture that fails to parse
/// is a broken fixture set, not a test failure to tolerate.
fn load(file: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(file);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", path.display()))
}

/// Full corpus check: for every fixture, parse, compare every member
/// against its corpus declaration, extract, and require byte-identical
/// content whose CRC-32 matches the directory.
#[test]
fn every_fixture_matches_its_corpus_entry() {
    for fixture in FIXTURES {
        let data = load(fixture.file);
        let arc = ZipArchive::new(&data).unwrap_or_else(|e| {
            panic!(
                "fixture {} ({}): failed to parse: {e}",
                fixture.file, fixture.why
            )
        });
        assert_eq!(
            arc.len(),
            fixture.members.len(),
            "fixture {} member count drifted from the corpus",
            fixture.file
        );
        for (entry, member) in arc.entries().iter().zip(fixture.members) {
            assert_eq!(entry.name(), member.name);
            assert_eq!(entry.method(), member.method);
            assert_eq!(entry.uses_data_descriptor(), member.descriptor);
            let body = member.body.bytes();
            assert_eq!(entry.uncompressed_size(), body.len() as u64);
            let bytes = arc.extract(entry).unwrap_or_else(|e| {
                panic!(
                    "fixture {} member {} failed to extract: {e}",
                    fixture.file, member.name
                )
            });
            assert_eq!(bytes, body);
            assert_eq!(pith_digest::crc32(&bytes), entry.crc32());
        }
    }
}

/// The directory holds exactly the corpus's files — nothing committed on
/// the side escapes the reference, nothing declared is missing on disk.
/// (Prose such as `PROVENANCE.md` is not a fixture.)
#[test]
fn fixture_directory_and_corpus_agree() {
    let mut committed: Vec<String> =
        std::fs::read_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"))
            .expect("fixtures directory exists")
            .map(|e| {
                e.expect("dirent")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|n| n.ends_with(".zip"))
            .collect();
    committed.sort();
    let mut declared: Vec<String> = FIXTURES.iter().map(|f| f.file.to_owned()).collect();
    declared.sort();
    assert_eq!(committed, declared);
}

/// The empty archive parses to an empty reader with the documented
/// absent-member behaviour, straight from the committed bytes.
#[test]
fn empty_fixture_reports_empty() {
    let data = load("empty.zip");
    let arc = ZipArchive::new(&data).expect("empty fixture parses");
    assert!(arc.is_empty());
    assert!(arc.entries().is_empty());
    assert!(arc.by_name("anything").is_none());
}

/// A stored fixture member extracts through the real on-disk archive,
/// with the extraction ceiling honoured (`Limits::default()` is the
/// documented default path for [`ZipArchive::extract`]).
#[test]
fn stored_fixture_extracts_under_the_default_ceiling() {
    let data = load("stored.zip");
    let arc = ZipArchive::new(&data).expect("stored fixture parses");
    let a = arc.by_name("a.txt").expect("a.txt present");
    assert_eq!(
        arc.extract(a).unwrap(),
        b"stored payload, no compression at all"
    );
}
