//! The C ABI surface of `pith-zip`: the entry points the Python
//! (ctypes), Node (koffi) and Go (cgo) SDKs bind through.
//!
//! The suite's FFI convention, defined by this module and mirrored by
//! every `pith-*` cdylib:
//!
//! * one flat set of `#[unsafe(no_mangle)] pub unsafe extern "C"`
//!   functions — raw pointers plus lengths, no structs across the
//!   boundary;
//! * every function returns a status code (see the constants below),
//!   never a `Result`, never a panic: a `panic = "abort"` cdylib must
//!   not be reachable from a foreign caller;
//! * an operation either hands ownership to the caller (and ships a
//!   matching `_free` — [`pith_zip_free`] here) or writes into
//!   caller-provided out-parameters;
//! * the `unsafe` allowance is confined to this module; every core
//!   module stays unsafe-free behind the crate-root `#![deny]`.
//!
//! The one operation is member enumeration: [`pith_zip_members`]
//! parses the archive, extracts and CRC-verifies every member, and
//! hands out the canonical member stream of [`crate::reference`].
//! The XML reader is deliberately not part of the FFI surface — no
//! `reference.json` vector exercises it and the SDK contract is the
//! member stream.

#![allow(unsafe_code)]

use crate::reader::ZipArchive;
use crate::reference::members_stream;

/// Status: success.
pub const PITH_OK: i32 = 0;
/// Status: a caller argument is invalid — a null pointer.
pub const PITH_E_INVALID: i32 = -1;
/// Status: the core reader refused the input (malformed ZIP: no end of
/// central directory, bad directory offset, unsupported feature, CRC
/// mismatch, or a truncated stream).
pub const PITH_E_REJECTED: i32 = -2;

/// Walks a ZIP archive and emits the canonical member stream the
/// `reference.json` vectors are defined over.
///
/// `data` points at `len` bytes of the complete archive file. On
/// success the function allocates a buffer, writes its address through
/// `out`, its length through `out_len`, and returns [`PITH_OK`]; the
/// caller owns the buffer and must release it with [`pith_zip_free`],
/// passing back the same pointer *and* length. The buffer layout is
/// the canonical member-stream serialization of [`crate::reference`]:
/// the `u32` member count, then per member the name, method, data
/// -descriptor flag, both sizes, CRC-32, content length and the
/// extracted content bytes.
///
/// # Safety
///
/// `data` must point to `len` readable bytes; `out` to one writable
/// pointer; `out_len` to one writable `usize`. All must stay valid for
/// the duration of the call; the function retains nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pith_zip_members(
    data: *const u8,
    len: usize,
    out: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    if data.is_null() || out.is_null() || out_len.is_null() {
        return PITH_E_INVALID;
    }
    let bytes = unsafe { core::slice::from_raw_parts(data, len) };
    match members_and_serialize(bytes) {
        Ok(stream) => {
            let len = stream.len();
            // Hand the exact-length buffer to the caller; `pith_zip_free`
            // reconstructs the boxed slice from the same length.
            let ptr = Box::into_raw(stream.into_boxed_slice());
            unsafe {
                *out = ptr.cast::<u8>();
                *out_len = len;
            }
            PITH_OK
        }
        Err(status) => status,
    }
}

/// Releases a buffer handed out by [`pith_zip_members`].
///
/// # Safety
///
/// `ptr` must be a pointer returned by [`pith_zip_members`] with the
/// `out_len` value that came back with it, and must not have been
/// released (or otherwise freed) before. Null is accepted and ignored,
/// so callers can free unconditionally on the error path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pith_zip_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    let slice = unsafe { core::slice::from_raw_parts_mut(ptr, len) };
    drop(unsafe { Box::from_raw(slice) });
}

/// The safe core of [`pith_zip_members`]: parse, extract every member,
/// then serialize canonically. Parsing and extraction failures map to
/// [`PITH_E_REJECTED`].
fn members_and_serialize(bytes: &[u8]) -> Result<Vec<u8>, i32> {
    let archive = ZipArchive::new(bytes).map_err(|_| PITH_E_REJECTED)?;
    members_stream(&archive).map_err(|_| PITH_E_REJECTED)
}

#[cfg(test)]
mod tests {
    use super::{
        PITH_E_INVALID, PITH_E_REJECTED, PITH_OK, members_and_serialize, pith_zip_free,
        pith_zip_members,
    };

    /// A committed conformance fixture, read as raw bytes.
    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!(
            "{}/tests/fixtures/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("fixture")
    }

    /// The canonical stream split back into per-member records: the
    /// header facts plus the content length (content itself skipped).
    fn walk(stream: &[u8]) -> Vec<(String, u16, bool, u64, u64, u32, u64)> {
        let be = |pos: usize, n: usize| -> u64 {
            let mut v = 0u64;
            for b in &stream[pos..pos + n] {
                v = (v << 8) | u64::from(*b);
            }
            v
        };
        let mut members = Vec::new();
        let mut pos = 4usize;
        for _ in 0..be(0, 4) {
            let name_len = be(pos, 4) as usize;
            pos += 4;
            let name = String::from_utf8(stream[pos..pos + name_len].to_vec()).expect("utf-8");
            pos += name_len;
            let method = be(pos, 2) as u16;
            pos += 2;
            let descriptor = stream[pos] == 1;
            pos += 1;
            let compressed = be(pos, 8);
            pos += 8;
            let uncompressed = be(pos, 8);
            pos += 8;
            let crc = be(pos, 4) as u32;
            pos += 4;
            let content_len = be(pos, 8);
            pos += 8;
            pos += content_len as usize;
            members.push((
                name,
                method,
                descriptor,
                compressed,
                uncompressed,
                crc,
                content_len,
            ));
        }
        assert_eq!(pos, stream.len(), "trailing bytes after the last member");
        members
    }

    /// A committed conformance fixture, walked end-to-end through the
    /// raw FFI: status OK, the length matches the serialization, the
    /// bytes match it, and the buffer round-trips through
    /// `pith_zip_free`.
    #[test]
    fn ffi_members_reproduces_the_canonical_stream() {
        let zip = fixture("stored.zip");
        let expected = members_and_serialize(&zip).expect("serialize");

        let mut out: *mut u8 = core::ptr::null_mut();
        let mut out_len: usize = 0;
        let status = unsafe { pith_zip_members(zip.as_ptr(), zip.len(), &mut out, &mut out_len) };
        assert_eq!(status, PITH_OK);
        assert_eq!(out_len, expected.len());
        let handed_back = unsafe { core::slice::from_raw_parts(out, out_len) };
        assert_eq!(handed_back, expected.as_slice());
        // The documented head: member count 2, then the first member's
        // name_len 5 and name "a.txt" (the recorded stored.zip facts).
        assert_eq!(&handed_back[..4], &[0, 0, 0, 2]);
        assert_eq!(&handed_back[4..13], b"\x00\x00\x00\x05a.txt");
        unsafe { pith_zip_free(out, out_len) };
    }

    /// Deflated members and data-descriptor members survive the FFI
    /// round-trip with every recorded fact intact (the descriptor.zip
    /// reference-vector facts).
    #[test]
    fn ffi_deflated_and_descriptor_members_round_trip() {
        let zip = fixture("descriptor.zip");
        let mut out: *mut u8 = core::ptr::null_mut();
        let mut out_len: usize = 0;
        let status = unsafe { pith_zip_members(zip.as_ptr(), zip.len(), &mut out, &mut out_len) };
        assert_eq!(status, PITH_OK);
        let stream = unsafe { core::slice::from_raw_parts(out, out_len) };

        let members = walk(stream);
        assert_eq!(members.len(), 2);
        assert_eq!(
            members[0],
            ("d1.bin".into(), 8, true, 55, 50, 0x1cf2_4857, 50)
        );
        assert_eq!(
            members[1],
            ("d2.bin".into(), 8, true, 57, 52, 0x0fe9_a563, 52)
        );
        unsafe { pith_zip_free(out, out_len) };
    }

    /// Null pointers are [`PITH_E_INVALID`]; a truncated archive is
    /// [`PITH_E_REJECTED`]; a null buffer is a legal free.
    #[test]
    fn ffi_refusals() {
        let zip = fixture("stored.zip");
        let mut out: *mut u8 = core::ptr::null_mut();
        let mut out_len: usize = 0;

        let null_data = unsafe { pith_zip_members(core::ptr::null(), 0, &mut out, &mut out_len) };
        assert_eq!(null_data, PITH_E_INVALID);

        let null_out = unsafe {
            pith_zip_members(zip.as_ptr(), zip.len(), core::ptr::null_mut(), &mut out_len)
        };
        assert_eq!(null_out, PITH_E_INVALID);

        let null_out_len =
            unsafe { pith_zip_members(zip.as_ptr(), zip.len(), &mut out, core::ptr::null_mut()) };
        assert_eq!(null_out_len, PITH_E_INVALID);

        // Twelve bytes of a real archive: local header only, no EOCD.
        let truncated = unsafe { pith_zip_members(zip.as_ptr(), 12, &mut out, &mut out_len) };
        assert_eq!(truncated, PITH_E_REJECTED);

        unsafe { pith_zip_free(core::ptr::null_mut(), 0) };

        // The safe core refuses garbage instead of panicking.
        assert_eq!(members_and_serialize(&[0u8; 16]), Err(PITH_E_REJECTED));
    }
}
