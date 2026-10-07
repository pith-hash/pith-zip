//! The canonical member-stream serialization the `reference.json`
//! vectors and the C ABI surface (`ffi`) are defined over.
//!
//! # Wire format
//!
//! Every integer is big-endian. The stream a member-emitting FFI call
//! hands out is exactly:
//!
//! 1. `member_count` as `u32`;
//! 2. per member, in central-directory order:
//!    - `name_len` as `u32`, followed by the member name as UTF-8
//!      bytes (the reader refuses non-UTF-8 names at
//!      [`ZipArchive::new`], so the name is always valid);
//!    - `method` as `u16` (`0` stored, `8` DEFLATE);
//!    - `data_descriptor` as one byte, `0` or `1`;
//!    - `compressed_size` as `u64`;
//!    - `uncompressed_size` as `u64`;
//!    - `crc32` as `u32`;
//!    - `content_len` as `u64`, followed by exactly that many bytes of
//!      the extracted member content (CRC-verified by extraction).
//!
//! The per-member `sha256` digests in `reference.json` cover the
//! `content` bytes alone; the member-emitting FFI operation returns the
//! whole stream, so the language SDKs re-derive every recorded field
//! from one buffer.
//!
//! This module is the single definition of that format: the vector
//! generator (`tools/gen-reference`) serializes its own records and
//! shares no code with the runtime, so nothing was moved here — the
//! generator stays untouched and the SDK-facing contract lives only in
//! this file.

use crate::reader::{ZipArchive, ZipEntry};
use pith_digest::{Error, Result};

/// Serializes one member — its central-directory facts plus the
/// extracted `content` — into the canonical stream fragment (see the
/// module docs for the exact layout).
pub fn member_bytes(entry: &ZipEntry<'_>, content: &[u8]) -> Result<Vec<u8>> {
    let name = entry.name().as_bytes();
    let name_len = u32::try_from(name.len())
        .map_err(|_| Error::BadValue("zip member name exceeds u32 length"))?;
    let content_len = u64::try_from(content.len())
        .map_err(|_| Error::BadValue("zip member content exceeds u64 length"))?;
    let mut out = Vec::with_capacity(4 + name.len() + 2 + 1 + 8 + 8 + 4 + 8 + content.len());
    out.extend_from_slice(&name_len.to_be_bytes());
    out.extend_from_slice(name);
    out.extend_from_slice(&entry.method().to_be_bytes());
    out.push(u8::from(entry.uses_data_descriptor()));
    out.extend_from_slice(&entry.compressed_size().to_be_bytes());
    out.extend_from_slice(&entry.uncompressed_size().to_be_bytes());
    out.extend_from_slice(&entry.crc32().to_be_bytes());
    out.extend_from_slice(&content_len.to_be_bytes());
    out.extend_from_slice(content);
    Ok(out)
}

/// Serializes a whole parsed archive into the canonical byte stream the
/// language SDKs receive: the `u32` member count followed by every
/// member in central-directory order, each extracted and CRC-verified.
///
/// Extraction failures (unsupported method, size mismatch, CRC
/// mismatch, truncated payload) propagate as the named
/// [`Error`](pith_digest::Error) — the caller, not this module, decides
/// the status code.
pub fn members_stream(archive: &ZipArchive<'_>) -> Result<Vec<u8>> {
    let count = u32::try_from(archive.len())
        .map_err(|_| Error::BadValue("zip member count exceeds u32"))?;
    let mut out = Vec::new();
    out.extend_from_slice(&count.to_be_bytes());
    for entry in archive.entries() {
        let content = archive.extract(entry)?;
        out.extend_from_slice(&member_bytes(entry, &content)?);
    }
    Ok(out)
}
