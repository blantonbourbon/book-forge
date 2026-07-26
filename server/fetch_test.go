package server

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/google/uuid"

	"github.com/blantonbourbon/book-forge/converter"
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

func TestExecuteCrawlKeepsImageFetchesWithinTotalByteBudget(t *testing.T) {
	fixtureRoot := t.TempDir()
	html := `<!doctype html>
<html lang="en">
  <body>
    <article>
      <h1>Byte Budget</h1>
      <p>Readable page with images.</p>
      <img src="/images/within-budget.svg" alt="within budget" />
      <img src="/images/too-large.svg" alt="too large" />
    </article>
  </body>
</html>`
	withinBudgetImage := `<svg xmlns="http://www.w3.org/2000/svg"><text>ok</text></svg>`
	tooLargeImage := `<svg xmlns="http://www.w3.org/2000/svg"><text>` + strings.Repeat("x", 256) + `</text></svg>`

	writeFixture(t, fixtureRoot, "html/byte-budget/index.html", html)
	writeFixture(t, fixtureRoot, "images/within-budget.svg", withinBudgetImage)
	writeFixture(t, fixtureRoot, "images/too-large.svg", tooLargeImage)

	maxTotalBytes := len([]byte(html)) + len([]byte(withinBudgetImage)) + 8
	summary := JobSummary{
		SourceURL: "https://example.test/byte-budget/index.html",
		Mode:      ModeCrawl,
		Metadata: converter.BookMetadata{
			Title:    "Byte Budget",
			Language: "en",
		},
		Options: APIOptions{
			IncludeImages: true,
		},
		Crawl: &CrawlSummary{
			PrefixURL:         "https://example.test/byte-budget/",
			MaxDepth:          0,
			MaxPages:          1,
			MaxTotalBytes:     maxTotalBytes,
			MaxDurationMillis: 30000,
		},
	}

	jobs := &JobManager{jobs: make(map[uuid.UUID]*JobRecord)}
	_, progress, err := executeCrawl(uuid.New(), jobs, &SharedFetcher{FixtureRoot: fixtureRoot}, nil, summary)
	if err != nil {
		t.Fatalf("executeCrawl returned error: %v", err)
	}
	if progress.BytesFetched > maxTotalBytes {
		t.Fatalf("BytesFetched = %d, want <= %d", progress.BytesFetched, maxTotalBytes)
	}
}

func writeFixture(t *testing.T, root, relPath, contents string) {
	t.Helper()
	path := filepath.Join(root, relPath)
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(contents), 0o644); err != nil {
		t.Fatal(err)
	}
}

func TestBrowserFetcherEnforcesMaxBytes(t *testing.T) {
	sidecar := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewEncoder(w).Encode(map[string]any{
			"ok":        true,
			"html":      strings.Repeat("x", 100),
			"finalUrl":  "https://8.8.8.8/",
			"mediaType": "text/html; charset=utf-8",
		})
	}))
	defer sidecar.Close()

	fetcher := NewBrowserFetcher(sidecar.URL)
	_, err := fetcher.Fetch("https://8.8.8.8/", 5*time.Second, 50)
	if err == nil {
		t.Fatal("expected response_too_large")
	}
	fetchErr, ok := err.(*FetchError)
	if !ok {
		t.Fatalf("error type = %T, want *FetchError", err)
	}
	if fetchErr.Code != "response_too_large" {
		t.Fatalf("code = %q, want response_too_large", fetchErr.Code)
	}
}

func TestBrowserFetcherRejectsUnsafeFinalURL(t *testing.T) {
	sidecar := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		_ = json.NewEncoder(w).Encode(map[string]any{
			"ok":        true,
			"html":      "<html></html>",
			"finalUrl":  "http://127.0.0.1/private",
			"mediaType": "text/html; charset=utf-8",
		})
	}))
	defer sidecar.Close()

	fetcher := NewBrowserFetcher(sidecar.URL)
	_, err := fetcher.Fetch("https://8.8.8.8/", 5*time.Second, 1024)
	if err == nil {
		t.Fatal("expected unsafe_url")
	}
	fetchErr, ok := err.(*FetchError)
	if !ok {
		t.Fatalf("error type = %T, want *FetchError", err)
	}
	if fetchErr.Code != "unsafe_url" {
		t.Fatalf("code = %q, want unsafe_url", fetchErr.Code)
	}
}

func TestCreateJobRejectsWhenConcurrencySaturated(t *testing.T) {
	jobs := NewJobManager()
	// Saturate the semaphore without starting real work.
	for i := 0; i < maxConcurrentJobs; i++ {
		jobs.sem <- struct{}{}
	}

	summary := JobSummary{
		SourceURL: "https://example.test/single-page/index.html",
		Mode:      ModeSingle,
		Metadata:  converter.BookMetadata{Title: "t", Language: "en"},
	}
	_, err := jobs.CreateJob(NewSharedFetcher(), nil, summary, "")
	if err == nil {
		t.Fatal("expected too_many_jobs error")
	}
	apiErr, ok := err.(*APIError)
	if !ok {
		t.Fatalf("error type = %T, want *APIError", err)
	}
	if apiErr.Status != http.StatusTooManyRequests || apiErr.Body.Code != "too_many_jobs" {
		t.Fatalf("got status=%d code=%q", apiErr.Status, apiErr.Body.Code)
	}
}

func TestJobOwnerIsolation(t *testing.T) {
	jobs := NewJobManager()
	id := uuid.New()
	jobs.mu.Lock()
	jobs.jobs[id] = &JobRecord{
		ID:         id,
		OwnerLogin: "alice",
		Status:     StatusCompleted,
		Summary:    JobSummary{Mode: ModeSingle},
		Artifact:   &Artifact{Filename: "book.epub", Bytes: []byte("epub")},
	}
	jobs.mu.Unlock()

	if _, err := jobs.GetResponse(id, "bob"); err == nil {
		t.Fatal("bob should not see alice's job")
	}
	if status, art := jobs.Artifact(id, "bob"); status != "" || art != nil {
		t.Fatalf("bob should not download alice's artifact, status=%q art=%v", status, art)
	}
	resp, err := jobs.GetResponse(id, "alice")
	if err != nil {
		t.Fatalf("alice GetResponse: %v", err)
	}
	if resp.ID != id.String() {
		t.Fatalf("unexpected id %q", resp.ID)
	}
	status, art := jobs.Artifact(id, "alice")
	if status != StatusCompleted || art == nil {
		t.Fatalf("alice Artifact status=%q art=%v", status, art)
	}
}

func TestCrawlFromRequestLowerBounds(t *testing.T) {
	source, err := url.Parse("https://example.com/book/")
	if err != nil {
		t.Fatal(err)
	}
	neg := -1
	zero := 0
	fields := []string{}
	_ = crawlFromRequest(&APICrawlOptions{
		MaxDepth:      &neg,
		MaxPages:      &zero,
		MaxTotalBytes: &zero,
	}, source, &fields)

	want := map[string]bool{
		"crawl.maxDepth":      false,
		"crawl.maxPages":      false,
		"crawl.maxTotalBytes": false,
	}
	for _, f := range fields {
		if _, ok := want[f]; ok {
			want[f] = true
		}
	}
	for field, found := range want {
		if !found {
			t.Fatalf("expected invalid field %q in %v", field, fields)
		}
	}
}
