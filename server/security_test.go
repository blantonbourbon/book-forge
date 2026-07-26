package server

import (
	"strings"
	"testing"
)

func TestValidateNetworkURLBlocksPrivateIPs(t *testing.T) {
	cases := []string{
		"http://127.0.0.1/",
		"http://127.0.0.1:8080/path",
		"https://10.0.0.1/",
		"http://192.168.1.1/admin",
		"http://172.16.0.5/",
		"http://169.254.169.254/latest/meta-data",
		"http://[::1]/",
		"http://localhost/",
		"http://0.0.0.0/",
	}
	for _, raw := range cases {
		err := ValidateNetworkURL(raw)
		if err == nil {
			t.Fatalf("ValidateNetworkURL(%q) = nil, want error", raw)
		}
		if sec, ok := err.(*SecurityError); ok && sec.Code != "unsafe_url" {
			t.Fatalf("ValidateNetworkURL(%q) code = %q, want unsafe_url", raw, sec.Code)
		}
	}
}

func TestValidateNetworkURLAcceptsPublicIPLiteral(t *testing.T) {
	// 8.8.8.8 is public; no DNS needed for IP literals.
	if err := ValidateNetworkURL("https://8.8.8.8/"); err != nil {
		t.Fatalf("ValidateNetworkURL(public IP) unexpected error: %v", err)
	}
}

func TestValidateNetworkURLRejectsCredentials(t *testing.T) {
	err := ValidateNetworkURL("https://user:pass@example.com/")
	if err == nil {
		t.Fatal("expected error for credentialed URL")
	}
	if !strings.Contains(err.Error(), "credentials") {
		t.Fatalf("unexpected error: %v", err)
	}
}
