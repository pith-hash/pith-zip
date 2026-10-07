// SPDX-License-Identifier: MIT
// Copyright (c) 2026 pith-hash
"use strict";

/**
 * pith-zip SDK: ZIP archive reading through koffi.
 *
 * The single Rust core (the `pith-zip` cdylib built by
 * `cargo build --release`) is loaded at runtime; koffi is the only
 * runtime dependency.
 *
 * Discovery order (the suite's cdylib convention):
 *
 *  1. `PITH_CDYLIB` — an explicit cdylib *file* path;
 *  2. `PITH_CDYLIB_DIR` — a *directory* scanned for the cdylib names
 *     (the CD pipeline points this at `target/release`);
 *  3. `prebuilds/` — the packaged npm layout the CD publish job
 *     assembles, flat and per `<os-arch>` (e.g. `linux-x64`);
 *  4. `<repo root>/target/release` — the repository working-tree
 *     layout, so a source checkout runs against a local cargo build
 *     with no configuration.
 *
 * The FFI surface is one member-enumeration operation plus one free:
 * `pith_zip_members` parses the archive, extracts and CRC-verifies
 * every member, and hands out the canonical member stream (member
 * count, then per member: name, method, data-descriptor flag, both
 * sizes, CRC-32, content length and the extracted content), and
 * `pith_zip_free` releases the handed-out buffer.
 */

const koffi = require("koffi");
const fs = require("node:fs");
const path = require("node:path");

const STATUS_OK = 0;
const STATUS_INVALID = -1;
const STATUS_REJECTED = -2;

/** Every cdylib file name cargo may drop into the build directory, per platform. */
const CDYLIB_NAMES = ["pith_zip.dll", "libpith_zip.so", "libpith_zip.dylib"];

const PKG_ROOT = path.join(__dirname);
const REPO_ROOT = path.resolve(__dirname, "..", "..");

/** FfiError: a non-zero status code came back from the cdylib. */
class FfiError extends Error {
  /**
   * @param {string} op the FFI operation name
   * @param {number} status the raw status code
   */
  constructor(op, status) {
    const kind = { [STATUS_INVALID]: "invalid argument", [STATUS_REJECTED]: "input rejected" }[status] ?? "unknown failure";
    super(`${op} failed: ${kind} (status ${status})`);
    this.name = "FfiError";
    /** The raw status code the FFI returned. */
    this.status = status;
  }
}

/**
 * Locates the cdylib through the suite's discovery chain.
 * @returns {string} an absolute path to the cdylib file
 * @throws {Error} when nothing is found
 */
function findCdylib() {
  const explicit = process.env.PITH_CDYLIB;
  if (explicit && fs.statSync(explicit, { throwIfNoEntry: false })?.isFile()) {
    return path.resolve(explicit);
  }
  /** @type {string[]} */
  const dirs = [];
  const envDir = process.env.PITH_CDYLIB_DIR;
  if (envDir) {
    dirs.push(envDir);
    if (!path.isAbsolute(envDir)) {
      dirs.push(path.join(REPO_ROOT, envDir));
    }
  }
  const osArch = `${process.platform}-${process.arch}`;
  dirs.push(path.join(PKG_ROOT, "prebuilds", osArch));
  dirs.push(path.join(PKG_ROOT, "prebuilds"));
  dirs.push(path.join(REPO_ROOT, "target", "release"));
  for (const dir of dirs) {
    for (const name of CDYLIB_NAMES) {
      const p = path.join(dir, name);
      if (fs.statSync(p, { throwIfNoEntry: false })?.isFile()) return p;
    }
  }
  throw new Error(
    "no pith-zip cdylib found (searched PITH_CDYLIB, PITH_CDYLIB_DIR, prebuilds/ and <repo>/target/release); " +
      "run `cargo build --release` first",
  );
}

let cached = undefined;

/**
 * Loads the cdylib and binds the exported symbols (lazily, once).
 * @returns {{members: Function, free: Function}}
 */
function loadLibrary() {
  if (cached) return cached;
  const lib = koffi.load(findCdylib());
  const members = lib.func("pith_zip_members", "int32_t", [
    "const uint8_t *",
    "size_t",
    koffi.out(koffi.pointer("void *")),
    koffi.out(koffi.pointer("size_t")),
  ]);
  const free = lib.func("void pith_zip_free(void *ptr, size_t len)");
  cached = { members, free };
  return cached;
}

/**
 * Enumerates a complete ZIP archive into the canonical member stream
 * the `reference.json` vectors are defined over. The handed-out
 * cdylib buffer is copied into a JS Buffer and released before
 * returning.
 *
 * @param {Buffer} data the complete archive file bytes
 * @returns {Buffer} the canonical stream (u32 member count + members)
 * @throws {FfiError} with `status === -2` for any malformed input
 */
function members(data) {
  if (!Buffer.isBuffer(data)) {
    throw new TypeError("data must be a Buffer");
  }
  const { members: membersFfi, free } = loadLibrary();
  const out = [null];
  const outLen = [0];
  const status = membersFfi(data, data.length, out, outLen);
  if (status !== STATUS_OK) {
    throw new FfiError("pith_zip_members", status);
  }
  try {
    // koffi.decode hands back a Uint8Array view over the external
    // buffer; copy it into a Buffer before the cdylib buffer is freed.
    return Buffer.from(koffi.decode(out[0], "uint8_t", Number(outLen[0])));
  } finally {
    free(out[0], Number(outLen[0]));
  }
}

/**
 * Re-expresses the canonical byte stream as plain objects.
 *
 * @param {Buffer} raw the canonical stream
 * @returns {{members: Array<{name: string, method: number, dataDescriptor: boolean,
 *   compressedSize: number, uncompressedSize: number, crc32: number, content: Buffer}>, raw: Buffer}}
 */
function parseMembers(raw) {
  if (!Buffer.isBuffer(raw) || raw.length < 4) {
    throw new TypeError("canonical stream is shorter than the 4-byte member count");
  }
  const count = raw.readUInt32BE(0);
  const parsed = [];
  let pos = 4;
  const need = (n) => {
    if (pos + n > raw.length) {
      throw new TypeError(`canonical stream is truncated at byte ${pos} (need ${n} more)`);
    }
  };
  for (let i = 0; i < count; i++) {
    need(4);
    const nameLen = raw.readUInt32BE(pos);
    pos += 4;
    need(nameLen + 15);
    const name = raw.subarray(pos, pos + nameLen).toString("utf8");
    pos += nameLen;
    const method = raw.readUInt16BE(pos);
    pos += 2;
    const descriptorByte = raw[pos];
    if (descriptorByte !== 0 && descriptorByte !== 1) {
      throw new TypeError(`unknown data-descriptor byte ${descriptorByte}`);
    }
    pos += 1;
    const compressedSize = raw.readBigUInt64BE(pos);
    pos += 8;
    const uncompressedSize = raw.readBigUInt64BE(pos);
    pos += 8;
    const crc32 = raw.readUInt32BE(pos);
    pos += 4;
    const contentLen = raw.readBigUInt64BE(pos);
    pos += 8;
    need(Number(contentLen));
    const content = Buffer.from(raw.subarray(pos, pos + Number(contentLen)));
    pos += Number(contentLen);
    parsed.push({
      name,
      method,
      dataDescriptor: descriptorByte === 1,
      compressedSize: Number(compressedSize),
      uncompressedSize: Number(uncompressedSize),
      crc32,
      content,
    });
  }
  if (pos !== raw.length) {
    throw new TypeError(`${raw.length - pos} trailing bytes after the last member`);
  }
  return { members: parsed, raw };
}

module.exports = {
  STATUS_OK,
  STATUS_INVALID,
  STATUS_REJECTED,
  CDYLIB_NAMES,
  FfiError,
  findCdylib,
  loadLibrary,
  members,
  parseMembers,
};
