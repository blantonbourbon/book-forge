package server

import (
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"

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
