# SPDX-License-Identifier: MIT
# Copyright (c) 2026 pith-hash
"""Hex-exact conformance: the committed reference vectors through ctypes.

Every vector in the repository-root ``reference.json`` is replayed
through the cdylib and compared field-exact — per-member SHA-256
content digests against the recorded values, plus every recorded
central-directory fact (name, method, data descriptor, both sizes,
CRC-32). The same vectors the Rust ``gen-reference verify`` gate and
the Node/Go SDKs check.
"""

from __future__ import annotations

import ctypes
import hashlib
import json
from pathlib import Path

import pytest

from pith_zip import Archive, FfiError, _load, find_cdylib, members, parse_members

REPO_ROOT = Path(__file__).resolve().parents[3]

VECTORS = json.loads((REPO_ROOT / "reference.json").read_text(encoding="utf-8"))["vectors"]


def test_cdylib_is_discoverable() -> None:
    path = find_cdylib()
    assert path.is_file(), path


@pytest.mark.parametrize("vector", VECTORS, ids=lambda v: v["name"])
def test_reference_vector_is_reproduced_hex_exact(vector: dict) -> None:
    name = vector["name"]
    data = (REPO_ROOT / "tests" / "fixtures" / name).read_bytes()

    raw = members(data)
    archive = parse_members(raw)

    assert len(archive.members) == vector["members"], name
    assert len(vector["entries"]) == vector["members"], name
    for entry, member in zip(vector["entries"], archive.members):
        assert member.name == entry["name"], name
        assert member.method == entry["method"], name
        assert member.data_descriptor is entry["data_descriptor"], name
        assert member.compressed_size == entry["compressed_size"], name
        assert member.uncompressed_size == entry["uncompressed_size"], name
        assert f"{member.crc32:08x}" == entry["crc32"], name
        assert hashlib.sha256(member.content).hexdigest() == entry["sha256"], name


def test_empty_archive_has_a_zero_member_count() -> None:
    archive = parse_members(members((REPO_ROOT / "tests" / "fixtures" / "empty.zip").read_bytes()))
    assert archive.members == ()
    assert len(archive.raw) == 4  # the u32 member count alone


def test_truncated_input_is_refused_not_crashing() -> None:
    data = (REPO_ROOT / "tests" / "fixtures" / "stored.zip").read_bytes()
    with pytest.raises(FfiError) as err:
        members(data[:12])
    assert err.value.status == -2


def test_garbage_input_is_refused() -> None:
    with pytest.raises(FfiError) as err:
        members(b"not a zip at all")
    assert err.value.status == -2


def test_null_pointer_is_invalid_not_crashing() -> None:
    # Wrapper-API misuse: the raw FFI is reachable with an explicit null
    # data pointer through the loaded library handle.
    out = ctypes.c_void_p()
    out_len = ctypes.c_size_t()
    status = _load().pith_zip_members(None, 0, ctypes.byref(out), ctypes.byref(out_len))
    assert status == -1


def test_full_stream_matches_a_rust_pinned_value() -> None:
    # stored.zip's full canonical-stream digest, derived from the Rust
    # build (the cdylib this test runs against) — this test fails loudly
    # even if reference.json were regenerated wrongly.
    data = (REPO_ROOT / "tests" / "fixtures" / "stored.zip").read_bytes()
    raw = members(data)
    assert hashlib.sha256(raw).hexdigest() == "39a31a066891585ed1d53671aee092bdfa6a02c9d98b05ac10ffe0a1e7a0d557"
    assert len(raw) == 3121
    archive: Archive = parse_members(raw)
    assert [m.name for m in archive.members] == ["a.txt", "b.bin"]
