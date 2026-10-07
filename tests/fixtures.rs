//! The fixture corpus: member-level truth for every archive under
//! `tests/fixtures/`.
//!
//! Each [`FIXTURES`] entry describes one committed `.zip` file by what the
//! reader must observe (member names, compression methods, data-descriptor
//! flags, uncompressed sizes, CRC-32s) and what extraction must produce
//! (the member bodies, either literal bytes or a deterministic
//! `i % modulus` pattern). The archive files themselves are the structural
//! truth — `tests/fixtures/PROVENANCE.md` records exactly how they were
//! built — and `tests/reference.rs` plus the `gen-reference` binary pin
//! both sides: any drift between a fixture's bytes and this corpus fails
//! the suite instead of silently re-baselining the reference.

/// The uncompressed content of one fixture member.
pub enum Body {
    /// Literal bytes, spelled out in source.
    Bytes(&'static [u8]),
    /// The `len` bytes `i % modulus` for `i in 0..len` — a
    /// pseudo-compressible corpus that never has to be spelled out.
    Pattern {
        /// Full length in bytes.
        len: usize,
        /// The byte value cycle `i % modulus`.
        modulus: u8,
    },
}

impl Body {
    /// The member's full uncompressed content.
    pub fn bytes(&self) -> Vec<u8> {
        match self {
            Body::Bytes(b) => b.to_vec(),
            Body::Pattern { len, modulus } => (0..*len)
                .map(|i| (i % usize::from(*modulus)) as u8)
                .collect(),
        }
    }
}

/// One member of a fixture archive, as the reader must observe it.
pub struct Member {
    /// Member name as stored in the central directory.
    pub name: &'static str,
    /// Compression method: `0` stored, `8` DEFLATE.
    pub method: u16,
    /// Whether the entry carries a data descriptor (flags bit 3).
    pub descriptor: bool,
    /// The member's uncompressed content.
    pub body: Body,
}

/// One committed archive under `tests/fixtures/`.
pub struct Fixture {
    /// File name inside `tests/fixtures/`.
    pub file: &'static str,
    /// What the fixture exercises, echoed into `reference.json`.
    pub why: &'static str,
    /// The archive's members, in central-directory order.
    pub members: &'static [Member],
}

/// The corpus, in deterministic order: `gen-reference` walks it in this
/// order and `reference.json` pins it in this order.
pub const FIXTURES: &[Fixture] = &[
    Fixture {
        file: "stored.zip",
        why: "two stored (method 0) members: short literal text and a 3 000-byte pattern",
        members: &[
            Member {
                name: "a.txt",
                method: 0,
                descriptor: false,
                body: Body::Bytes(b"stored payload, no compression at all"),
            },
            Member {
                name: "b.bin",
                method: 0,
                descriptor: false,
                body: Body::Pattern {
                    len: 3000,
                    modulus: 251,
                },
            },
        ],
    },
    Fixture {
        file: "deflated.zip",
        why: "deflated (method 8) members: one small, one spanning two stored DEFLATE blocks",
        members: &[
            Member {
                name: "s.txt",
                method: 8,
                descriptor: false,
                body: Body::Bytes(b"deflated payload, compressed with stored DEFLATE blocks"),
            },
            Member {
                name: "big.bin",
                method: 8,
                descriptor: false,
                body: Body::Pattern {
                    len: 70_000,
                    modulus: 253,
                },
            },
        ],
    },
    Fixture {
        file: "descriptor.zip",
        why: "flags-bit-3 members whose CRC and sizes trail the payload, with and without the optional 0x08074b50 signature",
        members: &[
            Member {
                name: "d1.bin",
                method: 8,
                descriptor: true,
                body: Body::Bytes(b"descriptor-backed deflate entry, signed descriptor"),
            },
            Member {
                name: "d2.bin",
                method: 8,
                descriptor: true,
                body: Body::Bytes(b"descriptor-backed deflate entry, unsigned descriptor"),
            },
        ],
    },
    Fixture {
        file: "mixed.zip",
        why: "stored, deflated and descriptor-backed members in one directory",
        members: &[
            Member {
                name: "m0.txt",
                method: 0,
                descriptor: false,
                body: Body::Bytes(b"mixed archive member zero, stored"),
            },
            Member {
                name: "m1.bin",
                method: 8,
                descriptor: false,
                body: Body::Pattern {
                    len: 1500,
                    modulus: 241,
                },
            },
            Member {
                name: "m2.bin",
                method: 8,
                descriptor: true,
                body: Body::Bytes(b"mixed archive member two, behind a signed descriptor"),
            },
        ],
    },
    Fixture {
        file: "comment.zip",
        why: "a 100-byte EOCD comment containing a PK\\x05\\x06 lookalike the scanner must skip",
        members: &[Member {
            name: "a.txt",
            method: 0,
            descriptor: false,
            body: Body::Bytes(b"with comment"),
        }],
    },
    Fixture {
        file: "empty.zip",
        why: "an archive with no members at all",
        members: &[],
    },
];
