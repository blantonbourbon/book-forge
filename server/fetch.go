package server

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/PuerkitoBio/goquery"
)

const maxRedirects = 5

type FetchedResponse struct {
	FinalURL  string
	MediaType string
	Bytes     []byte
}

func (r *FetchedResponse) Text() (string, error) {
	return string(r.Bytes), nil
}

type FetchError struct {
	Code    string
	Message string
}

func (e *FetchError) Error() string {
	return e.Message
}

func NewFetchError(code, message string) *FetchError {
	return &FetchError{Code: code, Message: message}
}

type Fetcher interface {
	Fetch(urlStr string, timeout time.Duration, maxBytes int) (*FetchedResponse, error)
}

type SharedFetcher struct {
	FixtureRoot string
	Client      *http.Client
}

func NewSharedFetcher() *SharedFetcher {
	return &SharedFetcher{
		FixtureRoot: findFixtureRoot(),
		Client: &http.Client{
			CheckRedirect: func(req *http.Request, via []*http.Request) error {
				return http.ErrUseLastResponse
			},
		},
	}
}

func findFixtureRoot() string {
	candidates := []string{
		"fixtures",
		"../../fixtures",
	}
	for _, c := range candidates {
		if info, err := os.Stat(c); err == nil && info.IsDir() {
			if abs, err := filepath.Abs(c); err == nil {
				return abs
			}
		}
	}
	return "fixtures"
}

func (f *SharedFetcher) Fetch(urlStr string, timeout time.Duration, maxBytes int) (*FetchedResponse, error) {
	u, err := url.Parse(urlStr)
	if err != nil {
		return nil, NewFetchError("fetch_failed", "Source URL could not be parsed.")
	}

	host := strings.ToLower(u.Hostname())
	if host == "example.test" {
		return f.fetchFixture(u, timeout, maxBytes)
	}
	return f.fetchHTTP(u, timeout, maxBytes)
}

func (f *SharedFetcher) fetchFixture(u *url.URL, timeout time.Duration, maxBytes int) (*FetchedResponse, error) {
	type result struct {
		resp *FetchedResponse
		err  error
	}
	ch := make(chan result, 1)
	go func() {
		resp, err := f.fetchFixtureImpl(u, maxBytes)
		ch <- result{resp, err}
	}()

	select {
	case r := <-ch:
		return r.resp, r.err
	case <-time.After(timeout):
		return nil, NewFetchError("fetch_timeout", "Fetching source content timed out.")
	}
}

func (f *SharedFetcher) fetchFixtureImpl(u *url.URL, maxBytes int) (*FetchedResponse, error) {
	currentURL := *u
	for redirectCount := 0; redirectCount <= maxRedirects; redirectCount++ {
		if err := ValidateNetworkURL(currentURL.String()); err != nil {
			return nil, NewFetchError("unsafe_url", err.Error())
		}

		if loc := fixtureRedirectLocation(&currentURL); loc != "" {
			if redirectCount == maxRedirects {
				return nil, NewFetchError("redirect_limit_exceeded", "Redirect handling exceeded the configured limit.")
			}
			resolved, err := currentURL.Parse(loc)
			if err != nil {
				return nil, NewFetchError("invalid_redirect", "Redirect target was not a valid URL.")
			}
			currentURL = *resolved
			continue
		}

		relPath := fixtureRelativePath(&currentURL)
		if relPath == "" {
			return nil, NewFetchError("fixture_not_found", "The deterministic fixture content was not available.")
		}
		fullPath := filepath.Join(f.FixtureRoot, relPath)

		bytes, err := os.ReadFile(fullPath)
		if err != nil {
			return nil, NewFetchError("fetch_failed", "The deterministic fixture content was not available.")
		}

		declaredBytes := fixtureDeclaredBytes(&currentURL)
		if declaredBytes > 0 && declaredBytes > maxBytes {
			return nil, NewFetchError("response_too_large", "Fetched content exceeded the configured byte limit.")
		}
		if len(bytes) > maxBytes {
			return nil, NewFetchError("response_too_large", "Fetched content exceeded the configured byte limit.")
		}

		return &FetchedResponse{
			FinalURL:  currentURL.String(),
			MediaType: mediaTypeForPath(fullPath),
			Bytes:     bytes,
		}, nil
	}

	return nil, NewFetchError("redirect_limit_exceeded", "Redirect handling exceeded the configured limit.")
}

func (f *SharedFetcher) fetchHTTP(u *url.URL, timeout time.Duration, maxBytes int) (*FetchedResponse, error) {
	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()
	return f.fetchHTTPImpl(ctx, u, maxBytes)
}

func (f *SharedFetcher) fetchHTTPImpl(ctx context.Context, u *url.URL, maxBytes int) (*FetchedResponse, error) {
	currentURL := *u

	for redirectCount := 0; redirectCount <= maxRedirects; redirectCount++ {
		vetted, err := resolveVettedAddrs(&currentURL)
		if err != nil {
			return nil, NewFetchError("unsafe_url", err.Error())
		}

		canonical := CanonicalDomainForOutboundRequest(&currentURL)
		if canonical != "" && currentURL.Hostname() != canonical {
			currentURL.Host = canonical
			if currentURL.Port() == "" {
				if currentURL.Scheme == "https" {
					currentURL.Host += ":443"
				} else {
					currentURL.Host += ":80"
				}
			}
		}

		req, err := http.NewRequestWithContext(ctx, "GET", currentURL.String(), nil)
		if err != nil {
			return nil, NewFetchError("fetch_failed", "Source content could not be fetched.")
		}
		req.Header.Set("User-Agent", "BookForge/0.1")

		client := f.Client
		if vetted != nil && len(vetted.Addresses) > 0 {
			client = clientWithDialer(vetted.Domain, vetted.Addresses, outboundPort(&currentURL))
		}

		resp, err := client.Do(req)
		if err != nil {
			if errors.Is(err, context.DeadlineExceeded) || errors.Is(err, context.Canceled) {
				return nil, NewFetchError("fetch_timeout", "Fetching source content timed out.")
			}
			return nil, NewFetchError("fetch_failed", "Source content could not be fetched.")
		}

		status := resp.StatusCode
		if status >= 300 && status < 400 {
			if redirectCount == maxRedirects {
				resp.Body.Close()
				return nil, NewFetchError("redirect_limit_exceeded", "Redirect handling exceeded the configured limit.")
			}
			loc := resp.Header.Get("Location")
			resp.Body.Close()
			if loc == "" {
				return nil, NewFetchError("invalid_redirect", "Redirect response did not include a valid target.")
			}
			resolved, err := currentURL.Parse(loc)
			if err != nil {
				return nil, NewFetchError("invalid_redirect", "Redirect target was not a valid URL.")
			}
			currentURL = *resolved
			continue
		}

		if status < 200 || status >= 300 {
			resp.Body.Close()
			return nil, NewFetchError("fetch_failed", fmt.Sprintf("Source returned HTTP status %d.", status))
		}

		if resp.ContentLength > int64(maxBytes) {
			resp.Body.Close()
			return nil, NewFetchError("response_too_large", "Fetched content exceeded the configured byte limit.")
		}

		mediaType := "application/octet-stream"
		if ct := resp.Header.Get("Content-Type"); ct != "" {
			if idx := strings.IndexByte(ct, ';'); idx >= 0 {
				mediaType = strings.TrimSpace(ct[:idx])
			} else {
				mediaType = strings.TrimSpace(ct)
			}
		}

		body, err := io.ReadAll(io.LimitReader(resp.Body, int64(maxBytes+1)))
		resp.Body.Close()
		if err != nil {
			if errors.Is(err, context.DeadlineExceeded) || errors.Is(err, context.Canceled) {
				return nil, NewFetchError("fetch_timeout", "Fetching source content timed out.")
			}
			return nil, NewFetchError("fetch_failed", "Source body could not be read.")
		}
		if len(body) > maxBytes {
			return nil, NewFetchError("response_too_large", "Fetched content exceeded the configured byte limit.")
		}

		return &FetchedResponse{
			FinalURL:  currentURL.String(),
			MediaType: mediaType,
			Bytes:     body,
		}, nil
	}

	return nil, NewFetchError("redirect_limit_exceeded", "Redirect handling exceeded the configured limit.")
}

func clientWithDialer(domain string, ips []net.IP, port string) *http.Client {
	dialer := &net.Dialer{Timeout: 10 * time.Second}
	return &http.Client{
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			return http.ErrUseLastResponse
		},
		Transport: &http.Transport{
			DialContext: func(ctx context.Context, network, addr string) (net.Conn, error) {
				addr = net.JoinHostPort(ips[0].String(), port)
				return dialer.DialContext(ctx, "tcp", addr)
			},
			MaxIdleConns:        100,
			MaxIdleConnsPerHost: 10,
			IdleConnTimeout:     90 * time.Second,
		},
	}
}

func outboundPort(u *url.URL) string {
	if port := u.Port(); port != "" {
		return port
	}
	if u.Scheme == "https" {
		return "443"
	}
	return "80"
}

func fixtureRelativePath(u *url.URL) string {
	path := strings.TrimPrefix(u.Path, "/")
	if path == "" {
		return ""
	}
	segments := strings.Split(path, "/")
	for _, seg := range segments {
		if seg == "" || unsafePathSegment(seg) {
			return ""
		}
	}
	if segments[0] == "images" {
		return filepath.Join(segments...)
	}
	return filepath.Join(append([]string{"html"}, segments...)...)
}

func unsafePathSegment(seg string) bool {
	lower := strings.ToLower(seg)
	return seg == "." || seg == ".." || strings.Contains(lower, "%2e") || strings.Contains(lower, "%2f") || strings.Contains(lower, `\`)
}

func mediaTypeForPath(path string) string {
	ext := strings.ToLower(filepath.Ext(path))
	switch ext {
	case ".html", ".htm":
		return "text/html; charset=utf-8"
	case ".svg":
		return "image/svg+xml"
	case ".png":
		return "image/png"
	case ".jpg", ".jpeg":
		return "image/jpeg"
	case ".webp":
		return "image/webp"
	default:
		return "application/octet-stream"
	}
}

func fixtureRedirectLocation(u *url.URL) string {
	switch u.Path {
	case "/redirects/to-safe":
		return "/single-page/index.html"
	case "/redirects/to-private":
		return "http://127.0.0.1:3100/private-target"
	case "/redirects/loop-a":
		return "/redirects/loop-b"
	case "/redirects/loop-b":
		return "/redirects/loop-a"
	}
	return ""
}

func fixtureDeclaredBytes(u *url.URL) int {
	if u.Path == "/oversized-slow/oversized.html" {
		return 10485761
	}
	return 0
}

func ExtractLinkURLs(htmlContent string, base *url.URL) []*url.URL {
	return extractURLs(htmlContent, base, "a[href]", "href")
}

func ExtractImageURLs(htmlContent string, base *url.URL) []*url.URL {
	return extractURLs(htmlContent, base, "img[src]", "src")
}

func extractURLs(htmlContent string, base *url.URL, selector, attr string) []*url.URL {
	doc, err := goquery.NewDocumentFromReader(strings.NewReader(htmlContent))
	if err != nil {
		return nil
	}

	var urls []*url.URL
	doc.Find(selector).Each(func(i int, s *goquery.Selection) {
		val, exists := s.Attr(attr)
		if !exists || val == "" || containsControlChar(val) {
			return
		}
		u, err := base.Parse(val)
		if err == nil {
			urls = append(urls, u)
		}
	})
	return urls
}

func containsControlChar(s string) bool {
	for _, ch := range s {
		if ch < 32 {
			return true
		}
	}
	return false
}

func IsHTMLike(mediaType string) bool {
	parts := strings.SplitN(mediaType, ";", 2)
	mt := strings.TrimSpace(strings.ToLower(parts[0]))
	switch mt {
	case "text/html", "application/xhtml+xml", "application/xml", "text/xml":
		return true
	}
	return false
}

func DefaultPrefixURL(sourceURL *url.URL) string {
	prefix := *sourceURL
	prefix.RawQuery = ""
	prefix.Fragment = ""
	if !strings.HasSuffix(prefix.Path, "/") {
		path := prefix.Path
		if idx := strings.LastIndexByte(path, '/'); idx > 0 {
			path = path[:idx+1]
		}
		prefix.Path = path
	}
	return prefix.String()
}

// ── Browser fetcher (CloakBrowser sidecar) ──

type BrowserFetcher struct {
	sidecarURL string
	client     *http.Client
}

func NewBrowserFetcher(sidecarURL string) *BrowserFetcher {
	return &BrowserFetcher{
		sidecarURL: strings.TrimRight(sidecarURL, "/"),
		client:     &http.Client{Timeout: 0},
	}
}

func (f *BrowserFetcher) Fetch(urlStr string, timeout time.Duration, maxBytes int) (*FetchedResponse, error) {
	type fetchRequest struct {
		URL string `json:"url"`
	}
	type fetchError struct {
		Code    string `json:"code"`
		Message string `json:"message"`
	}
	type fetchResponse struct {
		OK        bool        `json:"ok"`
		HTML      string      `json:"html,omitempty"`
		FinalURL  string      `json:"finalUrl,omitempty"`
		MediaType string      `json:"mediaType,omitempty"`
		Bytes     int         `json:"bytes,omitempty"`
		Error     *fetchError `json:"error,omitempty"`
	}

	body, _ := json.Marshal(fetchRequest{URL: urlStr})

	ctx, cancel := context.WithTimeout(context.Background(), timeout)
	defer cancel()

	req, err := http.NewRequestWithContext(ctx, "POST", f.sidecarURL+"/fetch", bytes.NewReader(body))
	if err != nil {
		return nil, NewFetchError("fetch_failed", "Browser fetch could not be prepared.")
	}
	req.Header.Set("Content-Type", "application/json")

	resp, err := f.client.Do(req)
	if err != nil {
		if errors.Is(err, context.DeadlineExceeded) {
			return nil, NewFetchError("fetch_timeout", "Browser fetch timed out.")
		}
		return nil, NewFetchError("fetch_failed", "Browser sidecar was not reachable.")
	}
	defer resp.Body.Close()

	var result fetchResponse
	if err := json.NewDecoder(resp.Body).Decode(&result); err != nil {
		return nil, NewFetchError("fetch_failed", "Browser sidecar returned an unexpected response.")
	}

	if !result.OK {
		code := "fetch_failed"
		message := "Browser fetch failed."
		if result.Error != nil {
			code = result.Error.Code
			message = result.Error.Message
		}
		return nil, NewFetchError(code, message)
	}

	finalURL := result.FinalURL
	if finalURL == "" {
		finalURL = urlStr
	}
	if err := ValidateNetworkURL(finalURL); err != nil {
		return nil, NewFetchError("unsafe_url", err.Error())
	}
	if maxBytes > 0 && len(result.HTML) > maxBytes {
		return nil, NewFetchError("response_too_large", "Fetched content exceeded the configured byte limit.")
	}
	mediaType := result.MediaType
	if mediaType == "" {
		mediaType = "text/html; charset=utf-8"
	}

	return &FetchedResponse{
		FinalURL:  finalURL,
		MediaType: mediaType,
		Bytes:     []byte(result.HTML),
	}, nil
}
