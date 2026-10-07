# Fixture provenance

Every `*.zip` in this directory is a deterministic archive built once by a
throwaway builder and committed byte-exactly; nothing here is produced by a
library compressor whose output could drift between toolchains or releases.
Member-level truth for each file lives in `../fixtures.rs`; the files on
disk are the structural truth. If either side changes, `gen-reference
verify` (CI) and `tests/reference.rs` fail — fixtures are never
regenerated in place to make a gate green without re-cutting
`reference.json` in the same commit.

## Builder semantics (byte layout, PKZIP APPNOTE.TXT)

- Local file header (30 bytes) + name + payload (`+` data descriptor for
  bit-3 members); central directory entry (46 bytes) + name per member;
  end-of-central-directory (22 bytes) + comment.
- All integers little-endian. Version needed/made = 20. MS-DOS time
  `0x7e21`, date `0x0121` in both header halves. Extra/comment lengths 0.
- Flags: bit 3 set iff the member is descriptor-backed; the local header's
  CRC/sizes are zeroed for those members and the record trails the payload,
  with (`0x08074b50`) or without its optional signature, exactly as
  declared in the corpus.
- `method 8` payloads are hand-rolled stored-block DEFLATE (BTYPE 00):
  a final `0x01` block for inputs up to 65 535 bytes, a non-final `0x00`
  block plus a final `0x01` block for the two-block case (`big.bin`,
  70 000 bytes, split at 65 535). This is legal DEFLATE whose bytes are
  fixed forever — no compressor is involved.
- `method 0` payloads are the member bytes verbatim.
- CRC-32 is the CRC-32 (IEEE) of the uncompressed member bytes.
- Member bodies are either literal ASCII or the deterministic pattern
  `i % modulus` for `i in 0..len` (`b.bin` 3000/251, `big.bin` 70000/253,
  `m1.bin` 1500/241).
- `comment.zip` carries a 100-byte EOCD comment containing the four bytes
  `50 4B 05 06` at offset 10 — a lookalike the EOCD scanner must reject
  via the comment-length cross-check.
- `empty.zip` is a valid archive with zero members (EOCD only, comment
  length 0).

## Regenerating

There is no committed generator on purpose: the fixtures are frozen bytes.
To rebuild one, follow the layout above and check the result against the
corpus in `../fixtures.rs` plus `reference.json` at the repo root
(`cargo run --bin gen-reference -- verify` must stay green, and the new
`reference.json` must be committed alongside the new bytes).
