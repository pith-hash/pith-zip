# SPDX-License-Identifier: MIT
# Copyright (c) 2026 pith-hash
"""pith-zip SDK: ZIP archive reading through ctypes.

The single Rust core (the ``pith-zip`` cdylib built by
``cargo build --release``) is loaded at runtime; this package carries
no third-party dependency — ``ctypes`` is the standard library.

Discovery order (the suite's cdylib convention):

1. ``PITH_CDYLIB`` — an explicit cdylib *file* path;
2. ``PITH_CDYLIB_DIR`` — a *directory* scanned for the cdylib names
   (the CD pipeline points this at ``target/release``);
3. the package directory itself (the built wheel ships the cdylib as
   package data);
4. ``<repo root>/target/release`` — the repository working-tree layout,
   so a source checkout runs against a local cargo build with no
   configuration.

The FFI surface is one member-enumeration operation plus one free:
``pith_zip_members`` parses the archive, extracts and CRC-verifies
every member, and hands out the canonical member stream (member count,
then per member: name, method, data-descriptor flag, both sizes,
CRC-32, content length and the extracted content), and
``pith_zip_free`` releases the handed-out buffer.
"""

from __future__ import annotations

import ctypes
import os
from dataclasses import dataclass
from pathlib import Path

__all__ = [
    "Archive",
    "Member",
    "FfiError",
    "LibraryNotFoundError",
    "find_cdylib",
    "members",
    "parse_members",
    "STATUS_OK",
    "STATUS_INVALID",
    "STATUS_REJECTED",
]

#: Status: success.
STATUS_OK = 0
#: Status: a caller argument is invalid (a null pointer).
STATUS_INVALID = -1
#: Status: the core reader refused the input (malformed ZIP).
STATUS_REJECTED = -2

#: Every cdylib file name cargo may drop into the build directory, per
#: platform (windows / linux / macOS).
CDYLIB_NAMES = ("pith_zip.dll", "libpith_zip.so", "libpith_zip.dylib")


@dataclass(frozen=True)
class Member:
    """One archive member, re-expressed from the canonical stream.

    ``content`` is the extracted (and CRC-verified) member body —
    exactly the bytes the per-member ``sha256`` digest in
    ``reference.json`` covers.
    """

    #: The member name as recorded in the central directory.
    name: str
    #: The compression method: ``0`` (stored) or ``8`` (DEFLATE).
    method: int
    #: Whether the entry carries a data descriptor (flags bit 3).
    data_descriptor: bool
    #: Payload length on disk, in bytes.
    compressed_size: int
    #: Decompressed length, in bytes.
    uncompressed_size: int
    #: CRC-32 of the uncompressed payload.
    crc32: int
    #: The extracted member content.
    content: bytes


@dataclass(frozen=True)
class Archive:
    """The whole archive, re-expressed from the canonical stream."""

    #: The members, in central-directory order.
    members: tuple[Member, ...]
    #: The canonical byte stream the FFI handed out.
    raw: bytes


class LibraryNotFoundError(OSError):
    """No cdylib was found through the discovery chain."""


class FfiError(Exception):
    """A non-zero status code came back from the cdylib."""

    def __init__(self, op: str, status: int) -> None:
        kind = {
            STATUS_INVALID: "invalid argument",
            STATUS_REJECTED: "input rejected",
        }.get(status, "unknown failure")
        super().__init__(f"{op} failed: {kind} (status {status})")
        #: The raw status code the FFI returned.
        self.status = status


def find_cdylib() -> Path:
    """Locates the cdylib through the suite's discovery chain."""
    explicit = os.environ.get("PITH_CDYLIB")
    if explicit:
        p = Path(explicit)
        if p.is_file():
            return p
    env_dir = os.environ.get("PITH_CDYLIB_DIR")
    candidates: list[Path] = []
    if env_dir:
        env_dir_path = Path(env_dir)
        candidates.append(env_dir_path)
        if not env_dir_path.is_absolute():
            # CD and local runs invoke tools from the repository root or
            # from sdk/<lang>; resolve the env value against both.
            candidates.append(Path.cwd() / env_dir_path)
            candidates.append(Path(__file__).resolve().parents[3] / env_dir_path)
    candidates.append(Path(__file__).resolve().parent)  # packaged wheel
    candidates.append(Path(__file__).resolve().parents[3] / "target" / "release")
    for directory in candidates:
        for name in CDYLIB_NAMES:
            p = directory / name
            if p.is_file():
                return p
    raise LibraryNotFoundError(
        "no pith-zip cdylib found (searched PITH_CDYLIB, PITH_CDYLIB_DIR, "
        "the package directory and <repo>/target/release); "
        "run `cargo build --release` first"
    )


_lib: ctypes.CDLL | None = None


def _load() -> ctypes.CDLL:
    global _lib
    if _lib is None:
        lib = ctypes.CDLL(str(find_cdylib()))
        lib.pith_zip_members.argtypes = [
            ctypes.c_void_p,  # data
            ctypes.c_size_t,  # len
            ctypes.POINTER(ctypes.c_void_p),  # out buffer
            ctypes.POINTER(ctypes.c_size_t),  # out length
        ]
        lib.pith_zip_members.restype = ctypes.c_int32
        lib.pith_zip_free.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
        lib.pith_zip_free.restype = None
        _lib = lib
    return _lib


def members(data: bytes) -> bytes:
    """Enumerates a complete ZIP archive into the canonical member
    stream the ``reference.json`` vectors are defined over.

    Every member is extracted and CRC-verified by the core before the
    stream is handed out. Raises :class:`FfiError` with
    ``status == STATUS_REJECTED`` for any malformed input — no end of
    central directory, a bad directory offset, an unsupported feature
    or a CRC mismatch — and ``status == STATUS_INVALID`` for a null
    argument; the core never panics through this boundary.
    """
    out = ctypes.c_void_p()
    out_len = ctypes.c_size_t()
    status = _load().pith_zip_members(data, len(data), ctypes.byref(out), ctypes.byref(out_len))
    if status != STATUS_OK:
        raise FfiError("pith_zip_members", status)
    try:
        return ctypes.string_at(out, out_len.value)
    finally:
        _load().pith_zip_free(out, out_len.value)


def _need(raw: bytes, pos: int, n: int) -> None:
    if pos + n > len(raw):
        raise ValueError(f"canonical stream is truncated at byte {pos} (need {n} more)")


def _be(raw: bytes, pos: int, n: int) -> tuple[int, int]:
    _need(raw, pos, n)
    return int.from_bytes(raw[pos : pos + n], "big"), pos + n


def parse_members(raw: bytes) -> Archive:
    """Re-expresses the canonical byte stream as an :class:`Archive`."""
    count, pos = _be(raw, 0, 4)
    parsed: list[Member] = []
    for _ in range(count):
        name_len, pos = _be(raw, pos, 4)
        _need(raw, pos, name_len + 15)
        name = raw[pos : pos + name_len].decode("utf-8")
        pos += name_len
        method, pos = _be(raw, pos, 2)
        if raw[pos] not in (0, 1):
            raise ValueError(f"unknown data-descriptor byte {raw[pos]}")
        descriptor = raw[pos] == 1
        pos += 1
        compressed_size, pos = _be(raw, pos, 8)
        uncompressed_size, pos = _be(raw, pos, 8)
        crc32, pos = _be(raw, pos, 4)
        content_len, pos = _be(raw, pos, 8)
        _need(raw, pos, content_len)
        content = raw[pos : pos + content_len]
        pos += content_len
        parsed.append(
            Member(
                name=name,
                method=method,
                data_descriptor=descriptor,
                compressed_size=compressed_size,
                uncompressed_size=uncompressed_size,
                crc32=crc32,
                content=content,
            )
        )
    if pos != len(raw):
        raise ValueError(f"{len(raw) - pos} trailing bytes after the last member")
    return Archive(members=tuple(parsed), raw=raw)
