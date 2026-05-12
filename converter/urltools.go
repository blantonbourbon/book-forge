package converter

import (
	"net/url"
	"strings"
)

func parseURL(raw string) (*url.URL, error) {
	return url.Parse(raw)
}

func sameOrigin(left, right *url.URL) bool {
	leftPort := left.Port()
	rightPort := right.Port()
	if leftPort == "" {
		leftPort = defaultPortForScheme(left.Scheme)
	}
	if rightPort == "" {
		rightPort = defaultPortForScheme(right.Scheme)
	}
	return strings.EqualFold(left.Scheme, right.Scheme) &&
		strings.EqualFold(left.Hostname(), right.Hostname()) &&
		leftPort == rightPort
}

func defaultPortForScheme(scheme string) string {
	switch strings.ToLower(scheme) {
	case "http":
		return "80"
	case "https":
		return "443"
	default:
		return ""
	}
}

func normalizePageURL(u *url.URL) string {
	nu := cloneURL(u)
	nu.Fragment = ""
	nu.RawQuery = ""
	normalizeDefaultPagePath(nu)
	return nu.String()
}

func normalizeResourceURL(u *url.URL) string {
	nu := cloneURL(u)
	nu.Fragment = ""
	return nu.String()
}

func urlWithoutFragment(u *url.URL) string {
	nu := cloneURL(u)
	nu.Fragment = ""
	return nu.String()
}

func defaultPrefixFor(startURL *url.URL) string {
	prefix := cloneURL(startURL)
	prefix.RawQuery = ""
	prefix.Fragment = ""

	path := prefix.Path
	if !strings.HasSuffix(path, "/") {
		if idx := strings.LastIndexByte(path, '/'); idx > 0 {
			path = path[:idx+1]
		} else {
			path = "/"
		}
	}
	prefix.Path = path
	return prefix.String()
}

func cloneURL(u *url.URL) *url.URL {
	nu := *u
	nu.User = nil
	return &nu
}

func normalizeDefaultPagePath(u *url.URL) {
	path := u.Path
	normalized := path

	for _, suffix := range []string{"/index.html", "/index.htm"} {
		if strings.HasSuffix(normalized, suffix) {
			normalized = normalized[:len(normalized)-len(suffix)+1]
			break
		}
	}

	if normalized == "" {
		normalized = "/"
	}

	if !strings.HasSuffix(normalized, "/") {
		lastSlash := strings.LastIndexByte(normalized, '/')
		lastSegment := normalized[lastSlash+1:]
		if !strings.Contains(lastSegment, ".") {
			normalized += "/"
		}
	}

	u.Path = normalized
}
