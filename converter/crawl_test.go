package converter

import (
	"net/url"
	"testing"
)

func TestResolveImageSrcHandlesRelativeURLs(t *testing.T) {
	page, err := url.Parse("https://example.com/articles/post.html")
	if err != nil {
		t.Fatal(err)
	}

	tests := []struct {
		name    string
		raw     string
		want    string
		wantErr bool
	}{
		{name: "absolute https", raw: "https://cdn.example.com/img.png", want: "https://cdn.example.com/img.png"},
		{name: "root-relative", raw: "/static/img.png", want: "https://example.com/static/img.png"},
		{name: "relative", raw: "img.png", want: "https://example.com/articles/img.png"},
		{name: "dot relative", raw: "./img.png", want: "https://example.com/articles/img.png"},
		{name: "protocol-relative", raw: "//cdn.example.com/img.png", want: "https://cdn.example.com/img.png"},
		{name: "data url rejected", raw: "data:image/png;base64,xx", wantErr: true},
		{name: "mailto rejected", raw: "mailto:foo@bar.com", wantErr: true},
		{name: "empty rejected", raw: "", wantErr: true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := resolveImageSrc(tt.raw, page)
			if tt.wantErr {
				if err == nil {
					t.Fatalf("resolveImageSrc(%q): expected error, got %q", tt.raw, got)
				}
				return
			}
			if err != nil {
				t.Fatalf("resolveImageSrc(%q): unexpected error %v", tt.raw, err)
			}
			if got.String() != tt.want {
				t.Fatalf("resolveImageSrc(%q) = %q, want %q", tt.raw, got, tt.want)
			}
		})
	}
}

func TestResolvePageLinkHandlesRelativeURLs(t *testing.T) {
	page, err := url.Parse("https://example.com/docs/intro.html")
	if err != nil {
		t.Fatal(err)
	}

	tests := []struct {
		name string
		raw  string
		want string
	}{
		{name: "absolute https", raw: "https://example.com/docs/next.html", want: "https://example.com/docs/next.html"},
		{name: "root-relative", raw: "/docs/next.html", want: "https://example.com/docs/next.html"},
		{name: "sibling", raw: "next.html", want: "https://example.com/docs/next.html"},
		{name: "fragment-only dropped", raw: "#section"},
		{name: "javascript dropped", raw: "javascript:alert(1)"},
		{name: "mailto dropped", raw: "mailto:foo@bar.com"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := resolvePageLink(tt.raw, page)
			if tt.want == "" {
				if got != nil {
					t.Fatalf("resolvePageLink(%q): expected nil, got %q", tt.raw, got)
				}
				return
			}
			if got == nil {
				t.Fatalf("resolvePageLink(%q): expected %q, got nil", tt.raw, tt.want)
			}
			if got.String() != tt.want {
				t.Fatalf("resolvePageLink(%q) = %q, want %q", tt.raw, got, tt.want)
			}
		})
	}
}
