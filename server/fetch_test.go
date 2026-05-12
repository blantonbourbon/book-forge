package server

import (
	"net/url"
	"testing"
)

func TestOutboundPortUsesSchemeDefaults(t *testing.T) {
	tests := []struct {
		name string
		raw  string
		want string
	}{
		{name: "https default", raw: "https://pdai.tech/", want: "443"},
		{name: "http default", raw: "http://example.com/book", want: "80"},
		{name: "explicit port", raw: "https://example.com:8443/book", want: "8443"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			parsed, err := url.Parse(tt.raw)
			if err != nil {
				t.Fatal(err)
			}
			if got := outboundPort(parsed); got != tt.want {
				t.Fatalf("outboundPort(%q) = %q, want %q", tt.raw, got, tt.want)
			}
		})
	}
}
