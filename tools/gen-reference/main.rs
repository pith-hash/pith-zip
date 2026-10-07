//! Regenerates and verifies `reference.json`, the hex-exact archive
//! reference this suite ships beside every SDK artifact.
//!
//! The corpus is the committed fixture set `tests/fixtures/*.zip` (member
//! truth in `tests/fixtures.rs`); this binary turns that corpus into a
//! language-neutral JSON document by *parsing and extracting every archive
//! through `pith-zip`*, checking each member against the corpus, and
//! pinning every member's content with its CRC-32 and SHA-256 digests.
//! The result is a fixed point: `reference.json` is exactly what the
//! current reader produces for the current fixtures, so any regression in
//! either shows up as a `verify` failure instead of a silently stale file.
//!
//! - `gen-reference gen` writes `reference.json` at the repository root.
//! - `gen-reference verify` recomputes it and fails on any drift; this is
//!   the mode the CI gate runs.
//!
//! The binary uses `std` (it touches the filesystem) but adds no
//! dependencies: the JSON emission is hand-rolled, and the digests are the
//! suite's own `pith_digest::{crc32, sha256}` — the same CRC-32 the reader
//! verifies every extracted member with.

#[path = "../../tests/fixtures.rs"]
mod fixtures;

use std::fs;
use std::path::{Path, PathBuf};

use fixtures::FIXTURES;
use pith_digest::{crc32, sha256};
use pith_zip::ZipArchive;

/// Where the reference file lives: the repository root, next to the crate
/// manifest, so the CD workflow can ship it with the SDK artifacts
/// regardless of the directory `cargo run` was invoked from.
fn reference_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("reference.json")
}

/// Where the committed archive fixtures live.
fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// Reads one fixture archive from `tests/fixtures/`. A missing or
/// unreadable fixture is a broken build, not a reference to write.
fn read_fixture(file: &str) -> Vec<u8> {
    let path = fixtures_dir().join(file);
    fs::read(&path).unwrap_or_else(|e| panic!("cannot read fixture {}: {e}", path.display()))
}

/// Parses one fixture archive. A fixture the reader refuses is a broken
/// build: the reference must never paper over a reader change.
fn parse_fixture<'a>(file: &str, data: &'a [u8]) -> ZipArchive<'a> {
    ZipArchive::new(data).unwrap_or_else(|e| panic!("fixture {file} failed to parse: {e}"))
}

/// Asserts one parsed archive matches its corpus entry member-by-member
/// (names, methods, descriptor flags, sizes, CRC-32s) and extracts every
/// member, requiring byte-identical content. Returns the extracted
/// contents in directory order.
fn extract_fixture(file: &str, arc: &ZipArchive<'_>, members: &[fixtures::Member]) -> Vec<Vec<u8>> {
    assert_eq!(
        arc.len(),
        members.len(),
        "fixture {file} holds {} members, corpus declares {}",
        arc.len(),
        members.len()
    );
    let mut out = Vec::with_capacity(members.len());
    for (entry, member) in arc.entries().iter().zip(members) {
        assert_eq!(
            entry.name(),
            member.name,
            "fixture {file} member name drifted from the corpus"
        );
        assert_eq!(
            entry.method(),
            member.method,
            "fixture {file} member {} method drifted from the corpus",
            member.name
        );
        assert_eq!(
            entry.uses_data_descriptor(),
            member.descriptor,
            "fixture {file} member {} data-descriptor flag drifted from the corpus",
            member.name
        );
        let body = member.body.bytes();
        assert_eq!(
            entry.uncompressed_size(),
            body.len() as u64,
            "fixture {file} member {} uncompressed size drifted from the corpus",
            member.name
        );
        let bytes = arc.extract(entry).unwrap_or_else(|e| {
            panic!(
                "fixture {file} member {} failed to extract: {e}",
                member.name
            )
        });
        assert_eq!(
            bytes, body,
            "fixture {file} member {} extracted to bytes that differ from the corpus",
            member.name
        );
        assert_eq!(
            crc32(&bytes),
            entry.crc32(),
            "fixture {file} member {} extracted CRC-32 disagrees with the directory",
            member.name
        );
        out.push(bytes);
    }
    out
}

/// The eight-digit lowercase hex of a CRC-32: the format-native digest
/// every extracted member is already verified against.
fn crc_hex(data: &[u8]) -> String {
    format!("{:08x}", crc32(data))
}

/// Lowercase hex of a byte slice (used for the SHA-256 content digest).
fn hex(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 2);
    for byte in data {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Escapes one string for a JSON string literal. The corpus strings are
/// plain ASCII names and prose, but the escaper is complete so the output
/// can never be corrupted by a future edit to either.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Total member count across the corpus — the number of pinned entries.
fn member_count() -> usize {
    FIXTURES.iter().map(|f| f.members.len()).sum()
}

/// The exact bytes of `reference.json` for the current fixtures and
/// reader: two-space indent, corpus order, one trailing newline. Nothing
/// here is sorted or deduplicated on purpose - the file is a transcript
/// of the corpus, in corpus order, so a diff against a previous commit
/// reads as a changelog of the fixture set.
fn reference_json() -> String {
    let mut out = String::with_capacity(16 * 1024);
    out.push_str("{\n");
    out.push_str("  \"schema\": 1,\n");
    out.push_str("  \"crate\": \"pith-zip\",\n");
    out.push_str(
        "  \"description\": \"hex-exact ZIP archive reference vectors: every \
tests/fixtures/*.zip archive parsed by pith-zip, each member checked against \
its corpus entry and pinned by CRC-32 and SHA-256 content digests\",\n",
    );
    out.push_str("  \"vectors\": [\n");
    for (vi, fixture) in FIXTURES.iter().enumerate() {
        let data = read_fixture(fixture.file);
        let arc = parse_fixture(fixture.file, &data);
        let contents = extract_fixture(fixture.file, &arc, fixture.members);
        out.push_str("    {\n");
        out.push_str(&format!(
            "      \"name\": \"{}\",\n",
            json_escape(fixture.file)
        ));
        out.push_str(&format!(
            "      \"why\": \"{}\",\n",
            json_escape(fixture.why)
        ));
        out.push_str(&format!("      \"members\": {},\n", fixture.members.len()));
        out.push_str("      \"entries\": [\n");
        for (mi, (entry, bytes)) in arc.entries().iter().zip(&contents).enumerate() {
            out.push_str("        {\n");
            out.push_str(&format!(
                "          \"name\": \"{}\",\n",
                json_escape(entry.name())
            ));
            out.push_str(&format!("          \"method\": {},\n", entry.method()));
            out.push_str(&format!(
                "          \"compressed_size\": {},\n",
                entry.compressed_size()
            ));
            out.push_str(&format!(
                "          \"uncompressed_size\": {},\n",
                entry.uncompressed_size()
            ));
            out.push_str(&format!("          \"crc32\": \"{}\",\n", crc_hex(bytes)));
            let digest = sha256(bytes)
                .unwrap_or_else(|e| panic!("fixture member {} sha256 failed: {e}", entry.name()));
            out.push_str(&format!(
                "          \"sha256\": \"{}\",\n",
                hex(digest.as_bytes())
            ));
            out.push_str(&format!(
                "          \"data_descriptor\": {}\n",
                entry.uses_data_descriptor()
            ));
            out.push_str("        }");
            if mi + 1 < arc.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("      ]\n");
        out.push_str("    }");
        if vi + 1 < FIXTURES.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("  ]\n");
    out.push_str("}\n");
    out
}

/// Writes `reference.json`. Returns the number of pinned members.
fn generate_at(path: &Path) -> std::io::Result<usize> {
    let json = reference_json();
    fs::write(path, json)?;
    Ok(member_count())
}

/// Recomputes the reference and compares it byte-for-byte with the
/// committed copy. `Ok(entries)` means the committed file is current.
fn verify_at(path: &Path) -> Result<usize, String> {
    let committed =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let recomputed = reference_json();
    if committed != recomputed {
        let first = committed
            .bytes()
            .zip(recomputed.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(committed.len().min(recomputed.len()));
        return Err(format!(
            "recomputed reference differs from {} (first difference at byte {})",
            path.display(),
            first
        ));
    }
    Ok(member_count())
}

/// The CLI body: the mode argument against the reference path. Returns the
/// process exit code so tests can exercise every branch in-process; `main`
/// is the only thing that actually exits.
fn run(path: &Path, mode: Option<&str>) -> i32 {
    match mode {
        Some("gen") => match generate_at(path) {
            Ok(entries) => {
                println!(
                    "wrote {} ({} fixtures, {} members)",
                    path.display(),
                    FIXTURES.len(),
                    entries
                );
                0
            }
            Err(e) => {
                eprintln!("gen-reference: {e}");
                1
            }
        },
        Some("verify") => match verify_at(path) {
            Ok(n) => {
                println!("reference.json is current ({n} members verified)");
                0
            }
            Err(e) => {
                eprintln!("reference.json is stale: {e}");
                1
            }
        },
        other => {
            eprintln!("usage: gen-reference <gen|verify> (got {other:?})");
            2
        }
    }
}

fn main() {
    std::process::exit(run(&reference_path(), std::env::args().nth(1).as_deref()));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch path unique to this test process, cleaned up by the caller.
    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("gen-reference-{}-{name}", std::process::id()))
    }

    #[test]
    fn json_escape_escapes_specials() {
        assert_eq!(json_escape("plain"), "plain");
        assert_eq!(json_escape("a\"b\\c\nd\te\rf"), "a\\\"b\\\\c\\nd\\te\\rf");
        assert_eq!(json_escape("\u{1}"), "\\u0001");
    }

    #[test]
    fn corpus_declares_the_committed_fixtures() {
        // Every corpus entry must have a real fixture file, and the
        // directory must hold nothing the corpus does not account for
        // (prose such as PROVENANCE.md is not a fixture).
        let mut committed: Vec<_> = fs::read_dir(fixtures_dir())
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
        let mut declared: Vec<_> = FIXTURES.iter().map(|f| f.file.to_owned()).collect();
        declared.sort();
        assert_eq!(committed, declared, "fixture files and corpus disagree");
        for fixture in FIXTURES {
            let data = read_fixture(fixture.file);
            assert!(
                !data.is_empty(),
                "fixture {} is empty on disk",
                fixture.file
            );
        }
    }

    #[test]
    fn every_fixture_matches_its_corpus_entry() {
        for fixture in FIXTURES {
            let data = read_fixture(fixture.file);
            let arc = parse_fixture(fixture.file, &data);
            extract_fixture(fixture.file, &arc, fixture.members);
        }
    }

    #[test]
    fn reference_pins_every_fixture_member() {
        let json = reference_json();
        assert_eq!(
            json.matches("\"name\":").count(),
            FIXTURES.len() + member_count()
        );
        assert_eq!(json.matches("\"sha256\":").count(), member_count());
        assert_eq!(json.matches("\"crc32\":").count(), member_count());
        // The empty archive is pinned with an empty member list.
        assert!(json.contains("\"name\": \"empty.zip\""));
        // Every corpus `why` is carried through verbatim.
        for fixture in FIXTURES {
            assert!(json.contains(&format!("\"why\": \"{}\"", json_escape(fixture.why))));
        }
    }

    #[test]
    fn generate_then_verify_round_trips() {
        let path = scratch("roundtrip.json");
        let entries = generate_at(&path).expect("write reference");
        assert_eq!(entries, member_count());
        assert_eq!(verify_at(&path), Ok(entries));
        let written = fs::read_to_string(&path).expect("read back");
        assert_eq!(written, reference_json());
        assert!(written.ends_with("}\n"));
        fs::remove_file(&path).ok();
    }

    #[test]
    fn verify_rejects_a_tampered_reference() {
        let path = scratch("tamper.json");
        let mut json = reference_json();
        // Corrupt one JSON key: any byte drift must fail the verify.
        let pos = json.find("\"sha256\"").expect("digest field present");
        json.replace_range(pos..pos + 10, "\"sha257\"");
        fs::write(&path, json).expect("write tampered reference");
        assert!(verify_at(&path).is_err());
        fs::remove_file(&path).ok();
    }

    #[test]
    fn verify_reports_a_missing_file() {
        let path = scratch("missing-does-not-exist.json");
        fs::remove_file(&path).ok();
        let err = verify_at(&path).expect_err("missing file must fail");
        assert!(err.contains("cannot read"));
    }

    #[test]
    fn run_dispatches_gen_verify_and_usage() {
        let path = scratch("run.json");
        fs::remove_file(&path).ok();
        assert_eq!(run(&path, Some("gen")), 0);
        assert!(path.exists());
        assert_eq!(run(&path, Some("verify")), 0);
        assert_eq!(run(&path, Some("polish")), 2);
        assert_eq!(run(&path, None), 2);
        fs::remove_file(&path).ok();
    }

    #[test]
    fn run_verify_fails_on_a_stale_reference() {
        let path = scratch("run-stale.json");
        fs::write(&path, "{\n  \"schema\": 0\n}\n").expect("write stale reference");
        assert_eq!(run(&path, Some("verify")), 1);
        fs::remove_file(&path).ok();
    }

    #[test]
    fn reference_path_lands_beside_the_manifest() {
        assert!(reference_path().is_absolute());
        assert_eq!(
            reference_path().file_name(),
            Some("reference.json".as_ref())
        );
    }

    #[test]
    fn fixtures_dir_lands_under_tests() {
        assert!(fixtures_dir().is_absolute());
        assert_eq!(fixtures_dir().file_name(), Some("fixtures".as_ref()));
    }

    #[test]
    #[should_panic(expected = "holds 2 members, corpus declares 0")]
    fn a_member_count_drift_is_a_broken_build() {
        // Extracting against the wrong corpus slice must panic loudly
        // rather than emit a reference that papers over the drift.
        let data = read_fixture("stored.zip");
        let arc = parse_fixture("stored.zip", &data);
        extract_fixture("stored.zip", &arc, &[]);
    }

    #[test]
    #[should_panic(expected = "failed to parse")]
    fn an_unparsable_fixture_is_a_broken_build() {
        let _ = parse_fixture("not-a-fixture.zip", b"junk that is not a zip");
    }

    #[test]
    fn run_gen_reports_an_unwritable_reference() {
        // `gen` into a directory path: the write fails, the exit code is
        // 1 and nothing panics.
        let path = scratch("gen-into-a-directory");
        fs::create_dir_all(&path).expect("create scratch dir");
        assert_eq!(run(&path, Some("gen")), 1);
        fs::remove_dir(&path).ok();
    }
}
