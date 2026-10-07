//! Edge-of-port error arms for the ZIP reader: refusals the ported
//! `tests/zip.rs` suite cannot reach with its builder, pinned here by
//! name — the EOCD candidate cut off mid-record, the ZIP64 sentinel in a
//! central entry, a directory whose declared size disagrees with its
//! contents, and the extraction ceiling on stored entries.

use pith_digest::Error;
use pith_inflate::Limits;
use pith_zip::ZipArchive;

/// Writes `v` little-endian into `out`.
fn le16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Writes `v` little-endian into `out`.
fn le32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Builds the smallest archive that parses past every earlier check: one
/// stored member, headers and directory written by hand. `cs_override`
/// replaces the central entry's compressed-size field (the ZIP64
/// sentinel test needs 0xFFFF_FFFF there while the local header and
/// payload stay real).
fn one_member_archive(cs_override: Option<u32>) -> Vec<u8> {
    let name = b"a.txt";
    let body = b"payload bytes";
    let crc = pith_digest::crc32(body);
    let cs = body.len() as u32;
    let us = body.len() as u32;

    let mut out = Vec::new();
    let local_offset = out.len() as u32;
    le32(&mut out, 0x0403_4b50);
    le16(&mut out, 20);
    le16(&mut out, 0); // flags
    le16(&mut out, 0); // method: stored
    le16(&mut out, 0x7e21);
    le16(&mut out, 0x0121);
    le32(&mut out, crc);
    le32(&mut out, cs);
    le32(&mut out, us);
    le16(&mut out, name.len() as u16);
    le16(&mut out, 0);
    out.extend_from_slice(name);
    out.extend_from_slice(body);

    let cd_offset = out.len() as u32;
    le32(&mut out, 0x0201_4b50);
    le16(&mut out, 20);
    le16(&mut out, 20);
    le16(&mut out, 0); // flags
    le16(&mut out, 0); // method
    le16(&mut out, 0x7e21);
    le16(&mut out, 0x0121);
    le32(&mut out, crc);
    le32(&mut out, cs_override.unwrap_or(cs));
    le32(&mut out, us);
    le16(&mut out, name.len() as u16);
    le16(&mut out, 0); // extra
    le16(&mut out, 0); // comment
    le16(&mut out, 0); // disk start
    le16(&mut out, 0); // internal attrs
    le32(&mut out, 0); // external attrs
    le32(&mut out, local_offset);
    out.extend_from_slice(name);
    let cd_size = out.len() as u32 - cd_offset;

    le32(&mut out, 0x0605_4b50);
    le16(&mut out, 0);
    le16(&mut out, 0);
    le16(&mut out, 1);
    le16(&mut out, 1);
    le32(&mut out, cd_size);
    le32(&mut out, cd_offset);
    le16(&mut out, 0);
    out
}

/// An EOCD signature too close to EOF to hold its comment-length field
/// is a truncation, not a magic error: the record was recognised, the
/// bytes just are not there.
#[test]
fn eocd_signature_cut_before_comment_length_is_truncated() {
    assert_eq!(
        ZipArchive::new(b"PK\x05\x06").unwrap_err(),
        Error::truncated("end of central directory", 22, 4)
    );
}

/// A central entry carrying the ZIP64 sentinel in its compressed-size
/// field is refused by name at parse time.
#[test]
fn zip64_central_entry_is_refused() {
    let zip = one_member_archive(Some(0xFFFF_FFFF));
    assert_eq!(
        ZipArchive::new(&zip).unwrap_err(),
        Error::Unsupported("zip64 central directory entry")
    );
}

/// Leftover bytes between the last directory entry and the EOCD — a
/// declared directory size that disagrees with its contents — are a
/// named bad value.
#[test]
fn padded_central_directory_is_bad_value() {
    let mut zip = one_member_archive(None);
    let eocd = zip
        .windows(4)
        .rposition(|w| w == [0x50, 0x4b, 0x05, 0x06])
        .expect("archive has an eocd");
    // Splice four junk bytes between the directory's last entry and the
    // EOCD, then declare them part of the directory.
    let junk = [0x00, 0x00, 0x00, 0x00];
    zip.splice(eocd..eocd, junk);
    let eocd_at = eocd + junk.len();
    let size = u32::from_le_bytes(zip[eocd_at + 12..eocd_at + 16].try_into().expect("size"));
    zip[eocd_at + 12..eocd_at + 16].copy_from_slice(&(size + 4).to_le_bytes());
    assert_eq!(
        ZipArchive::new(&zip).unwrap_err(),
        Error::BadValue("zip central directory size")
    );
}

/// The extraction ceiling applies to stored entries too: an archive
/// claiming more stored bytes than `limits.max_output` allows is
/// `TooLarge`, never an oversized allocation.
#[test]
fn stored_entry_over_the_ceiling_is_too_large() {
    let zip = one_member_archive(None);
    let arc = ZipArchive::new(&zip).expect("parse");
    let entry = arc.by_name("a.txt").expect("entry present");
    let limits = Limits {
        max_output: 4,
        max_input: usize::MAX,
    };
    assert_eq!(
        arc.extract_with(entry, &limits).unwrap_err(),
        Error::too_large("zip entry", 4)
    );
}
