package server

import (
	"context"
	"net"
	"net/url"
	"strings"
	"time"
)

const dnsLookupTimeout = 2 * time.Second

type SecurityError struct {
	Code    string
	Message string
}

func (e *SecurityError) Error() string {
	return e.Message
}

func ValidateNetworkURL(rawURL string) error {
	u, err := url.Parse(rawURL)
	if err != nil {
		return &SecurityError{Code: "unsafe_url", Message: "Only HTTP and HTTPS source URLs are supported."}
	}
	_, err = resolveVettedAddrs(u)
	return err
}

type VettedResolvedAddrs struct {
	Domain    string
	Addresses []net.IP
}

func resolveVettedAddrs(u *url.URL) (*VettedResolvedAddrs, error) {
	if err := validateURLWithoutDNS(u); err != nil {
		return nil, err
	}

	domain := publicDomainForDNS(u)
	if domain == "" {
		return nil, nil
	}

	port := u.Port()
	if port == "" {
		if u.Scheme == "https" {
			port = "443"
		} else {
			port = "80"
		}
	}

	ips, err := lookupHostWithTimeout(domain, dnsLookupTimeout)
	if err != nil || len(ips) == 0 {
		return nil, &SecurityError{Code: "unsafe_url", Message: "DNS lookup did not complete safely for the requested host."}
	}

	vetted := &VettedResolvedAddrs{Domain: domain}
	for _, ip := range ips {
		if err := ensurePublicIP(ip); err != nil {
			return nil, err
		}
		vetted.Addresses = append(vetted.Addresses, ip)
	}

	return vetted, nil
}

func lookupHostWithTimeout(host string, timeout time.Duration) ([]net.IP, error) {
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()

	addrs, err := net.DefaultResolver.LookupIPAddr(ctx, host)
	if err != nil {
		return nil, err
	}
	ips := make([]net.IP, 0, len(addrs))
	for _, addr := range addrs {
		ips = append(ips, addr.IP)
	}
	return ips, nil
}

func validateURLWithoutDNS(u *url.URL) error {
	if u.Scheme != "http" && u.Scheme != "https" {
		return &SecurityError{Code: "unsafe_url", Message: "Only HTTP and HTTPS source URLs are supported."}
	}

	if u.User != nil {
		return &SecurityError{Code: "unsafe_url", Message: "URLs with embedded credentials are not allowed."}
	}

	host := u.Hostname()
	if host == "" {
		return &SecurityError{Code: "unsafe_url", Message: "Source URLs must include a host."}
	}

	if ip := net.ParseIP(host); ip != nil {
		return ensurePublicIP(ip)
	}

	normalized := normalizeDomain(host)
	if normalized == "" {
		return &SecurityError{Code: "unsafe_url", Message: "Source URLs must include a host."}
	}
	if isBlockedHostname(normalized) {
		return &SecurityError{Code: "unsafe_url", Message: "Localhost and metadata-service targets are not allowed."}
	}
	if ip := net.ParseIP(normalized); ip != nil {
		return ensurePublicIP(ip)
	}

	return nil
}

func publicDomainForDNS(u *url.URL) string {
	host := u.Hostname()
	if net.ParseIP(host) != nil {
		return ""
	}
	normalized := CanonicalDomainForOutboundRequest(u)
	if normalized == "" {
		return ""
	}
	if isFixtureDomain(normalized) || net.ParseIP(normalized) != nil {
		return ""
	}
	return normalized
}

func CanonicalDomainForOutboundRequest(u *url.URL) string {
	host := u.Hostname()
	if net.ParseIP(host) != nil {
		return ""
	}
	return normalizeDomain(host)
}

func normalizeDomain(domain string) string {
	return strings.ToLower(strings.TrimSuffix(domain, "."))
}

func isFixtureDomain(domain string) bool {
	return domain == "example.test"
}

func isBlockedHostname(domain string) bool {
	if domain == "localhost" || domain == "localhost.localdomain" {
		return true
	}
	if strings.HasSuffix(domain, ".localhost") || strings.HasSuffix(domain, ".localhost.localdomain") {
		return true
	}
	switch domain {
	case "metadata", "metadata.google.internal", "169.254.169.254":
		return true
	}
	return false
}

func ensurePublicIP(ip net.IP) error {
	if ip4 := ip.To4(); ip4 != nil {
		return ensurePublicIPv4(ip4)
	}
	return ensurePublicIPv6(ip)
}

func ensurePublicIPv4(ip net.IP) error {
	if ip.IsLoopback() || ip.IsPrivate() || ip.IsLinkLocalUnicast() ||
		ip.IsLinkLocalMulticast() || ip.IsMulticast() || ip.IsUnspecified() {
		return &SecurityError{Code: "unsafe_url", Message: "Private, local, link-local, metadata, and multicast targets are not allowed."}
	}

	octets := ip.To4()
	if octets[0] == 0 || octets[0] >= 240 {
		return &SecurityError{Code: "unsafe_url", Message: "Private, local, link-local, metadata, and multicast targets are not allowed."}
	}
	if octets[0] == 100 && octets[1] >= 64 && octets[1] <= 127 {
		return &SecurityError{Code: "unsafe_url", Message: "Private, local, link-local, metadata, and multicast targets are not allowed."}
	}
	if octets[0] == 169 && octets[1] == 254 {
		return &SecurityError{Code: "unsafe_url", Message: "Private, local, link-local, metadata, and multicast targets are not allowed."}
	}
	if octets[0] == 192 && octets[1] == 0 && octets[2] == 0 {
		return &SecurityError{Code: "unsafe_url", Message: "Private, local, link-local, metadata, and multicast targets are not allowed."}
	}
	if octets[0] == 198 && (octets[1] == 18 || octets[1] == 19) {
		return &SecurityError{Code: "unsafe_url", Message: "Private, local, link-local, metadata, and multicast targets are not allowed."}
	}

	return nil
}

func ensurePublicIPv6(ip net.IP) error {
	if ip4 := ip.To4(); ip4 != nil {
		return ensurePublicIPv4(ip4)
	}

	if ip.IsLoopback() || ip.IsUnspecified() || ip.IsMulticast() ||
		ip.IsLinkLocalUnicast() || ip.IsLinkLocalMulticast() {
		return &SecurityError{Code: "unsafe_url", Message: "Private, local, link-local, metadata, and multicast targets are not allowed."}
	}

	segments := []uint16{
		uint16(ip[0])<<8 | uint16(ip[1]),
		uint16(ip[2])<<8 | uint16(ip[3]),
		uint16(ip[4])<<8 | uint16(ip[5]),
		uint16(ip[6])<<8 | uint16(ip[7]),
		uint16(ip[8])<<8 | uint16(ip[9]),
		uint16(ip[10])<<8 | uint16(ip[11]),
		uint16(ip[12])<<8 | uint16(ip[13]),
		uint16(ip[14])<<8 | uint16(ip[15]),
	}

	if (segments[0] & 0xfe00) == 0xfc00 {
		return &SecurityError{Code: "unsafe_url", Message: "Private, local, link-local, metadata, and multicast targets are not allowed."}
	}

	return nil
}
