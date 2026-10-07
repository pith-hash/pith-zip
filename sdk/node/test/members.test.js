// SPDX-License-Identifier: MIT
// Copyright (c) 2026 pith-hash
"use strict";

// Hex-exact conformance: the committed reference vectors through koffi.
// Every vector in the repository-root reference.json is replayed through
// the cdylib and compared field-exact — per-member SHA-256 content
// digests against the recorded values, plus every recorded
// central-directory fact. The same vectors the Rust gen-reference verify
// gate and the Python/Go SDKs check.

const test = require("node:test");
const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const koffi = require("koffi");
const path = require("node:path");

const { FfiError, findCdylib, loadLibrary, members, parseMembers } = require("../index.js");

const REPO_ROOT = path.resolve(__dirname, "..", "..", "..");

const VECTORS = JSON.parse(fs.readFileSync(path.join(REPO_ROOT, "reference.json"), "utf8")).vectors;

test("cdylib is discoverable", () => {
  assert.ok(fs.statSync(findCdylib()).isFile());
});

for (const vector of VECTORS) {
  test(`reference vector ${vector.name} is reproduced hex-exact`, () => {
    const data = fs.readFileSync(path.join(REPO_ROOT, "tests", "fixtures", vector.name));

    const raw = members(data);
    const archive = parseMembers(raw);

    assert.equal(archive.members.length, vector.members, vector.name);
    assert.equal(vector.entries.length, vector.members, vector.name);
    const pairs = vector.entries.map((entry, i) => [entry, archive.members[i]]);
    for (const [entry, member] of pairs) {
      assert.equal(member.name, entry.name, vector.name);
      assert.equal(member.method, entry.method, vector.name);
      assert.equal(member.dataDescriptor, entry.data_descriptor, vector.name);
      assert.equal(member.compressedSize, entry.compressed_size, vector.name);
      assert.equal(member.uncompressedSize, entry.uncompressed_size, vector.name);
      assert.equal(member.crc32.toString(16).padStart(8, "0"), entry.crc32, vector.name);
      assert.equal(crypto.createHash("sha256").update(member.content).digest("hex"), entry.sha256, vector.name);
    }
  });
}

test("empty archive has a zero member count", () => {
  const raw = members(fs.readFileSync(path.join(REPO_ROOT, "tests", "fixtures", "empty.zip")));
  const archive = parseMembers(raw);
  assert.deepEqual(archive.members, []);
  assert.equal(archive.raw.length, 4); // the u32 member count alone
});

test("truncated input is refused, not crashing", () => {
  const data = fs.readFileSync(path.join(REPO_ROOT, "tests", "fixtures", "stored.zip"));
  assert.throws(() => members(data.subarray(0, 12)), (err) => {
    assert.ok(err instanceof FfiError);
    assert.equal(err.status, -2);
    return true;
  });
});

test("garbage input is refused, not crashing", () => {
  assert.throws(() => members(Buffer.from("not a zip at all")), (err) => {
    assert.ok(err instanceof FfiError);
    assert.equal(err.status, -2);
    return true;
  });
});

test("null pointer is invalid, not crashing", () => {
  // Wrapper-API misuse: the raw FFI is reachable with an explicit null
  // data pointer through a direct koffi binding.
  const lib = koffi.load(findCdylib());
  const raw = lib.func("pith_zip_members", "int32_t", [
    "const uint8_t *",
    "size_t",
    koffi.out(koffi.pointer("void *")),
    koffi.out(koffi.pointer("size_t")),
  ]);
  const out = [null];
  const outLen = [0];
  const status = raw(null, 0, out, outLen);
  assert.equal(status, -1);
});

test("full stream matches a rust-pinned value", () => {
  // stored.zip's full canonical-stream digest, derived from the Rust
  // build (the cdylib this test runs against); this test fails loudly
  // even if reference.json were regenerated wrongly.
  const data = fs.readFileSync(path.join(REPO_ROOT, "tests", "fixtures", "stored.zip"));
  const raw = members(data);
  assert.equal(crypto.createHash("sha256").update(raw).digest("hex"), "39a31a066891585ed1d53671aee092bdfa6a02c9d98b05ac10ffe0a1e7a0d557");
  assert.equal(raw.length, 3121);
  const archive = parseMembers(raw);
  assert.deepEqual(archive.members.map((m) => m.name), ["a.txt", "b.bin"]);
});
