// SPDX-License-Identifier: MIT
// Copyright (c) 2026 pith-hash

package pithzip

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"
)

// repoRoot resolves the repository root relative to this package
// (sdk/go -> sdk -> repo root), the anchor for reference.json and the
// committed fixtures.
func repoRoot(t *testing.T) string {
	t.Helper()
	root, err := filepath.Abs(filepath.Join("..", ".."))
	if err != nil {
		t.Fatal(err)
	}
	if st, err := os.Stat(filepath.Join(root, "reference.json")); err != nil || st.IsDir() {
		t.Fatalf("reference.json not found at %s", root)
	}
	return root
}

// referenceEntry is one recorded member of a reference vector.
type referenceEntry struct {
	Name             string `json:"name"`
	Method           uint16 `json:"method"`
	CompressedSize   uint64 `json:"compressed_size"`
	UncompressedSize uint64 `json:"uncompressed_size"`
	Crc32            string `json:"crc32"`
	Sha256           string `json:"sha256"`
	DataDescriptor   bool   `json:"data_descriptor"`
}

// referenceVector is one committed reference vector.
type referenceVector struct {
	Name    string           `json:"name"`
	Members int              `json:"members"`
	Entries []referenceEntry `json:"entries"`
}

// reference parses the committed reference.json.
func reference(t *testing.T) []referenceVector {
	t.Helper()
	raw, err := os.ReadFile(filepath.Join(repoRoot(t), "reference.json"))
	if err != nil {
		t.Fatal(err)
	}
	var parsed struct {
		Vectors []referenceVector `json:"vectors"`
	}
	if err := json.Unmarshal(raw, &parsed); err != nil {
		t.Fatal(err)
	}
	return parsed.Vectors
}

// TestReferenceVectorsHexExact replays every committed reference.json
// vector through the cdylib and compares field-exact: per-member
// SHA-256 content digests against the recorded values, plus every
// recorded central-directory fact — the same vectors the Rust
// gen-reference verify gate and the Python/Node SDKs check.
func TestReferenceVectorsHexExact(t *testing.T) {
	for _, want := range reference(t) {
		t.Run(want.Name, func(t *testing.T) {
			data, err := os.ReadFile(filepath.Join(repoRoot(t), "tests", "fixtures", want.Name))
			if err != nil {
				t.Fatal(err)
			}
			raw, err := Members(data)
			if err != nil {
				t.Fatalf("Members(%s): %v", want.Name, err)
			}
			archive, err := ParseMembers(raw)
			if err != nil {
				t.Fatal(err)
			}
			if len(archive.Members) != want.Members {
				t.Fatalf("%s: %d members, want %d", want.Name, len(archive.Members), want.Members)
			}
			if len(want.Entries) != want.Members {
				t.Fatalf("%s: reference lists %d entries, want %d", want.Name, len(want.Entries), want.Members)
			}
			for i, entry := range want.Entries {
				got := archive.Members[i]
				if got.Name != entry.Name {
					t.Errorf("%s[%d]: name %q, want %q", want.Name, i, got.Name, entry.Name)
				}
				if got.Method != entry.Method {
					t.Errorf("%s[%d]: method %d, want %d", want.Name, i, got.Method, entry.Method)
				}
				if got.DataDescriptor != entry.DataDescriptor {
					t.Errorf("%s[%d]: data descriptor %v, want %v", want.Name, i, got.DataDescriptor, entry.DataDescriptor)
				}
				if got.CompressedSize != entry.CompressedSize || got.UncompressedSize != entry.UncompressedSize {
					t.Errorf("%s[%d]: sizes %d/%d, want %d/%d", want.Name, i,
						got.CompressedSize, got.UncompressedSize, entry.CompressedSize, entry.UncompressedSize)
				}
				if crc := fmt.Sprintf("%08x", got.Crc32); crc != entry.Crc32 {
					t.Errorf("%s[%d]: crc32 %s, want %s", want.Name, i, crc, entry.Crc32)
				}
				digest := sha256.Sum256(got.Content)
				if gotHash := hex.EncodeToString(digest[:]); gotHash != entry.Sha256 {
					t.Errorf("%s[%d]: sha256 %s, want %s", want.Name, i, gotHash, entry.Sha256)
				}
			}
		})
	}
}

// TestStoredZipPinnedDigest pins stored.zip's full canonical-stream
// digest, derived from the Rust build; the binding fails loudly even
// if reference.json were regenerated wrongly.
func TestStoredZipPinnedDigest(t *testing.T) {
	data, err := os.ReadFile(filepath.Join(repoRoot(t), "tests", "fixtures", "stored.zip"))
	if err != nil {
		t.Fatal(err)
	}
	raw, err := Members(data)
	if err != nil {
		t.Fatal(err)
	}
	digest := sha256.Sum256(raw)
	const want = "39a31a066891585ed1d53671aee092bdfa6a02c9d98b05ac10ffe0a1e7a0d557"
	if got := hex.EncodeToString(digest[:]); got != want {
		t.Errorf("stored.zip: digest %s, want %s", got, want)
	}
	if len(raw) != 3121 {
		t.Errorf("stored.zip: stream length %d, want 3121", len(raw))
	}
	archive, err := ParseMembers(raw)
	if err != nil {
		t.Fatal(err)
	}
	if len(archive.Members) != 2 || archive.Members[0].Name != "a.txt" || archive.Members[1].Name != "b.bin" {
		t.Errorf("stored.zip: members %v, want [a.txt b.bin]", archive.Members)
	}
}

// TestTruncatedInputIsRefused checks the reader's refusal path on a
// twelve-byte cut of a real archive: a status code, never a crash.
func TestTruncatedInputIsRefused(t *testing.T) {
	data, err := os.ReadFile(filepath.Join(repoRoot(t), "tests", "fixtures", "stored.zip"))
	if err != nil {
		t.Fatal(err)
	}
	_, err = Members(data[:12])
	var ffi *FfiError
	if e, ok := err.(*FfiError); ok {
		ffi = e
	} else {
		t.Fatalf("want FfiError, got %v", err)
	}
	if ffi.Status != StatusRejected {
		t.Errorf("want StatusRejected, got %d", ffi.Status)
	}
}

// TestGarbageInputIsRefused checks the refusal path on non-archive
// bytes: a status code, never a crash.
func TestGarbageInputIsRefused(t *testing.T) {
	_, err := Members([]byte("not a zip at all"))
	var ffi *FfiError
	if e, ok := err.(*FfiError); ok {
		ffi = e
	} else {
		t.Fatalf("want FfiError, got %v", err)
	}
	if ffi.Status != StatusRejected {
		t.Errorf("want StatusRejected, got %d", ffi.Status)
	}
}

// TestNullPointerIsInvalid exercises the null-data-pointer refusal
// through the public API: Members(nil) hands the FFI a null pointer.
func TestNullPointerIsInvalid(t *testing.T) {
	_, err := Members(nil)
	var ffi *FfiError
	if e, ok := err.(*FfiError); ok {
		ffi = e
	} else {
		t.Fatalf("want FfiError, got %v", err)
	}
	if ffi.Status != StatusInvalid {
		t.Errorf("want StatusInvalid, got %d", ffi.Status)
	}
}
