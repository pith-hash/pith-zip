// SPDX-License-Identifier: MIT
// Copyright (c) 2026 pith-hash

// Package pithzip provides Go bindings for the pith-zip Rust cdylib:
// ZIP archive reading into the canonical member stream.
//
// The single Rust core (built by `cargo build --release`) is loaded at
// runtime; the package carries zero module dependencies. On unix the
// cdylib is opened with dlopen through cgo, on Windows with
// LoadLibrary through the standard syscall package — both resolve the
// library through the same discovery chain, so `go build ./... &&
// go test ./...` works unchanged on every OS the CD matrix builds.
//
// Discovery order (the suite's cdylib convention):
//
//  1. PITH_CDYLIB — an explicit cdylib file path;
//  2. PITH_CDYLIB_DIR — a directory scanned for the cdylib names (the
//     CD pipeline points this at target/release);
//  3. <repo root>/target/release — the repository working-tree layout,
//     anchored at this package's source directory, so a source
//     checkout runs against a local cargo build unconfigured.
//
// The FFI surface is one member-enumeration operation plus one free:
// pith_zip_members parses the archive, extracts and CRC-verifies every
// member, and hands out the canonical member stream (member count,
// then per member: name, method, data-descriptor flag, both sizes,
// CRC-32, content length and the extracted content), and pith_zip_free
// releases the handed-out buffer.
package pithzip

import (
	"encoding/binary"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"sync"
	"unsafe"
)

// Status codes returned by the cdylib's C ABI.
const (
	// StatusOK: success.
	StatusOK int32 = 0
	// StatusInvalid: a caller argument is invalid (a null pointer).
	StatusInvalid int32 = -1
	// StatusRejected: the core reader refused the input (malformed
	// ZIP: no end of central directory, bad directory offset,
	// unsupported feature, or CRC mismatch).
	StatusRejected int32 = -2
)

// cdylibNames are the file names cargo may drop into the build
// directory, per platform (windows / linux / macOS).
var cdylibNames = []string{"pith_zip.dll", "libpith_zip.so", "libpith_zip.dylib"}

// FfiError reports a non-zero status code from the cdylib.
type FfiError struct {
	// Op is the FFI operation name.
	Op string
	// Status is the raw status code the FFI returned.
	Status int32
}

func (e *FfiError) Error() string {
	kind := "unknown failure"
	switch e.Status {
	case StatusInvalid:
		kind = "invalid argument"
	case StatusRejected:
		kind = "input rejected"
	}
	return fmt.Sprintf("%s failed: %s (status %d)", e.Op, kind, e.Status)
}

// FindCdylib locates the cdylib through the suite's discovery chain.
func FindCdylib() (string, error) {
	if p := os.Getenv("PITH_CDYLIB"); p != "" {
		if st, err := os.Stat(p); err == nil && st.Mode().IsRegular() {
			return filepath.Abs(p)
		}
	}
	_, thisFile, _, ok := runtime.Caller(0)
	if !ok {
		return "", fmt.Errorf("pithzip: cannot locate the package source directory")
	}
	pkgDir := filepath.Dir(thisFile)
	repoRoot := filepath.Dir(filepath.Dir(pkgDir)) // sdk/go -> sdk -> repo root

	var dirs []string
	if env := os.Getenv("PITH_CDYLIB_DIR"); env != "" {
		dirs = append(dirs, env)
		if !filepath.IsAbs(env) {
			dirs = append(dirs, filepath.Join(repoRoot, env))
		}
	}
	dirs = append(dirs, filepath.Join(repoRoot, "target", "release"))
	for _, dir := range dirs {
		for _, name := range cdylibNames {
			p := filepath.Join(dir, name)
			if st, err := os.Stat(p); err == nil && st.Mode().IsRegular() {
				return p, nil
			}
		}
	}
	return "", fmt.Errorf(
		"pithzip: no cdylib found (searched PITH_CDYLIB, PITH_CDYLIB_DIR and <repo>/target/release); run `cargo build --release` first",
	)
}

// locate resolves the cdylib path once per process.
var locate = sync.OnceValues(FindCdylib)

// Member is one archive member, re-expressed from the canonical byte
// stream: the central-directory facts plus the extracted content.
type Member struct {
	// Name is the member name as recorded in the central directory.
	Name string
	// Method is the compression method: 0 (stored) or 8 (DEFLATE).
	Method uint16
	// DataDescriptor reports flags bit 3 (the local header's CRC and
	// sizes are zero; the values trail the payload).
	DataDescriptor bool
	// CompressedSize is the payload length on disk, in bytes.
	CompressedSize uint64
	// UncompressedSize is the decompressed length, in bytes.
	UncompressedSize uint64
	// Crc32 is the CRC-32 of the uncompressed payload.
	Crc32 uint32
	// Content is the extracted (and CRC-verified) member body —
	// exactly the bytes the per-member sha256 digest covers.
	Content []byte
}

// Archive is the whole archive, re-expressed from the canonical byte
// stream.
type Archive struct {
	// Members lists every member in central-directory order.
	Members []Member
	// Raw is the canonical byte stream the FFI handed out.
	Raw []byte
}

// Members enumerates a complete ZIP archive into the canonical member
// stream the reference.json vectors are defined over. The returned
// slice is a Go copy; the handed-out cdylib buffer is released before
// returning.
func Members(data []byte) ([]byte, error) {
	libPath, err := locate()
	if err != nil {
		return nil, err
	}
	var out *byte
	var outLen uintptr
	var dataPtr *byte
	if len(data) > 0 {
		dataPtr = &data[0]
	}
	status, err := ffiMembers(libPath, dataPtr, len(data), &out, &outLen)
	if err != nil {
		return nil, err
	}
	if status != StatusOK {
		return nil, &FfiError{Op: "pith_zip_members", Status: status}
	}
	buf := make([]byte, outLen)
	copy(buf, unsafe.Slice(out, outLen))
	ffiFree(libPath, out, outLen)
	return buf, nil
}

// ParseMembers re-expresses the canonical byte stream as an Archive.
func ParseMembers(raw []byte) (Archive, error) {
	if len(raw) < 4 {
		return Archive{}, fmt.Errorf("pithzip: canonical stream is shorter than the 4-byte member count")
	}
	count := binary.BigEndian.Uint32(raw)
	pos := 4
	need := func(n int) error {
		if pos+n > len(raw) {
			return fmt.Errorf("pithzip: canonical stream is truncated at byte %d (need %d more)", pos, n)
		}
		return nil
	}
	members := make([]Member, 0, count)
	for i := uint32(0); i < count; i++ {
		if err := need(4); err != nil {
			return Archive{}, err
		}
		nameLen := int(binary.BigEndian.Uint32(raw[pos:]))
		pos += 4
		if err := need(nameLen + 15); err != nil {
			return Archive{}, err
		}
		name := string(raw[pos : pos+nameLen])
		pos += nameLen
		method := binary.BigEndian.Uint16(raw[pos:])
		pos += 2
		switch raw[pos] {
		case 0, 1:
		default:
			return Archive{}, fmt.Errorf("pithzip: unknown data-descriptor byte %d", raw[pos])
		}
		descriptor := raw[pos] == 1
		pos++
		compressed := binary.BigEndian.Uint64(raw[pos:])
		pos += 8
		uncompressed := binary.BigEndian.Uint64(raw[pos:])
		pos += 8
		crc := binary.BigEndian.Uint32(raw[pos:])
		pos += 4
		contentLen := int(binary.BigEndian.Uint64(raw[pos:]))
		pos += 8
		if err := need(contentLen); err != nil {
			return Archive{}, err
		}
		content := make([]byte, contentLen)
		copy(content, raw[pos:pos+contentLen])
		pos += contentLen
		members = append(members, Member{
			Name:             name,
			Method:           method,
			DataDescriptor:   descriptor,
			CompressedSize:   compressed,
			UncompressedSize: uncompressed,
			Crc32:            crc,
			Content:          content,
		})
	}
	if pos != len(raw) {
		return Archive{}, fmt.Errorf("pithzip: %d trailing bytes after the last member", len(raw)-pos)
	}
	return Archive{Members: members, Raw: raw}, nil
}
