package server

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestStaticTargetRejectsAbsolutePathEscape(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "index.html"), []byte("<html></html>"), 0o644); err != nil {
		t.Fatal(err)
	}
	rootAbs, err := filepath.Abs(root)
	if err != nil {
		t.Fatal(err)
	}

	// Paths that must be rejected (traversal / encoded dots).
	mustReject := []string{
		"../../../etc/passwd",
		"..%2fetc%2fpasswd",
		"/..//etc/passwd",
	}
	for _, requestPath := range mustReject {
		target, err := staticTarget(root, requestPath)
		if err == nil {
			t.Fatalf("staticTarget(%q) = %q without error, want rejection", requestPath, target)
		}
		apiErr, ok := err.(*APIError)
		if !ok {
			t.Fatalf("staticTarget(%q) error type = %T, want *APIError", requestPath, err)
		}
		if apiErr.Body.Code != "static_path_rejected" && apiErr.Body.Code != "static_asset_not_found" {
			t.Fatalf("staticTarget(%q) code = %q, want rejection", requestPath, apiErr.Body.Code)
		}
	}

	// Absolute-looking URLs must never escape the static root or serve host files.
	// After containment, they may 404 or SPA-fallback to index under root.
	for _, requestPath := range []string{"//etc/passwd", "/etc/passwd"} {
		target, err := staticTarget(root, requestPath)
		if err != nil {
			apiErr, ok := err.(*APIError)
			if !ok {
				t.Fatalf("staticTarget(%q) error type = %T, want *APIError", requestPath, err)
			}
			if apiErr.Body.Code != "static_path_rejected" && apiErr.Body.Code != "static_asset_not_found" {
				t.Fatalf("staticTarget(%q) code = %q", requestPath, apiErr.Body.Code)
			}
			continue
		}
		rel, relErr := filepath.Rel(rootAbs, target)
		if relErr != nil || strings.HasPrefix(rel, "..") {
			t.Fatalf("staticTarget(%q) escaped root to %q", requestPath, target)
		}
		if target == "/etc/passwd" || strings.HasPrefix(target, "/etc/") {
			t.Fatalf("staticTarget(%q) resolved to host path %q", requestPath, target)
		}
	}
}

func TestStaticTargetServesContainedAsset(t *testing.T) {
	root := t.TempDir()
	assetDir := filepath.Join(root, "assets")
	if err := os.MkdirAll(assetDir, 0o755); err != nil {
		t.Fatal(err)
	}
	content := []byte("body{}")
	if err := os.WriteFile(filepath.Join(assetDir, "app.css"), content, 0o644); err != nil {
		t.Fatal(err)
	}

	target, err := staticTarget(root, "/assets/app.css")
	if err != nil {
		t.Fatalf("staticTarget returned error: %v", err)
	}
	got, err := os.ReadFile(target)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != string(content) {
		t.Fatalf("served content = %q, want %q", got, content)
	}

	rootAbs, err := filepath.Abs(root)
	if err != nil {
		t.Fatal(err)
	}
	rel, err := filepath.Rel(rootAbs, target)
	if err != nil || strings.HasPrefix(rel, "..") {
		t.Fatalf("target escaped root: %q under %q", target, rootAbs)
	}
}

func TestStaticTargetFallsBackToIndex(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, "index.html"), []byte("<html>ok</html>"), 0o644); err != nil {
		t.Fatal(err)
	}
	target, err := staticTarget(root, "/")
	if err != nil {
		t.Fatalf("staticTarget(/) error: %v", err)
	}
	if filepath.Base(target) != "index.html" {
		t.Fatalf("target = %q, want index.html", target)
	}
}
