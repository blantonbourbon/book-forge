package converter

import (
	"archive/zip"
	"bytes"
	"io"
	"net/url"
	"reflect"
	"strings"
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

func TestCollectImageResourcesSuppressesTimeLimitWarning(t *testing.T) {
	page, err := url.Parse("https://example.com/articles/post.html")
	if err != nil {
		t.Fatal(err)
	}

	timeLimit := CrawlTimeLimitFailure
	fetchErr := "fetch_timeout: read tcp ..."

	lookup := map[string]*resourceSource{
		"https://example.com/articles/skipped.png": {
			mediaType: "application/octet-stream",
			failure:   &timeLimit,
		},
		"https://example.com/articles/failed.png": {
			mediaType: "application/octet-stream",
			failure:   &fetchErr,
		},
	}

	imageSources := []ImageSource{{
		URL:    page,
		Images: []string{"skipped.png", "failed.png", "missing.png"},
	}}

	var warnings []ConversionWarning
	warningKeys := make(map[string]bool)
	resources, paths := collectImageResources(imageSources, lookup, &warnings, warningKeys)

	if len(resources) != 0 {
		t.Fatalf("expected no packaged resources (all failed), got %d", len(resources))
	}
	if len(paths) != 0 {
		t.Fatalf("expected no packaged paths, got %d", len(paths))
	}

	if len(warnings) != 2 {
		t.Fatalf("expected 2 warnings (failed.png + missing.png), got %d: %+v", len(warnings), warnings)
	}
	for _, w := range warnings {
		if w.Affected != nil && *w.Affected == "https://example.com/articles/skipped.png" {
			t.Fatalf("did not expect a warning for the time-limit-skipped image, got %+v", w)
		}
	}
}

func TestConvertCrawlDoesNotFloodWarningsForOmittedLinks(t *testing.T) {
	html := `<!doctype html>
<html lang="en">
  <body>
    <article>
      <h1>Start</h1>
      <p>Readable crawl start content.</p>
      <a href="one.html">One</a>
      <a href="two.html">Two</a>
      <a href="three.html">Three</a>
    </article>
  </body>
</html>`

	result, err := ConvertCrawl(CrawlInput{
		StartURL: "https://example.com/book/index.html",
		Pages: []CrawlPage{{
			URL:  "https://example.com/book/index.html",
			HTML: &html,
		}},
		Metadata: BookMetadata{
			Title:    "Test Crawl",
			Language: "en",
		},
		Options: ConversionOptions{IncludeImages: false},
		Crawl: CrawlOptions{
			PrefixURL:         "https://example.com/book/",
			MaxDepth:          1,
			MaxPages:          2,
			MaxTotalBytes:     1024 * 1024,
			MaxDurationMillis: 30000,
		},
	})
	if err != nil {
		t.Fatalf("ConvertCrawl returned error: %v", err)
	}

	if got := warningCount(result.Warnings, "page_fetch_failed"); got != 0 {
		t.Fatalf("expected omitted links not to be reported as fetch failures, got %d warnings: %+v", got, result.Warnings)
	}
	if got := warningCount(result.Warnings, "crawl_page_limit"); got > 1 {
		t.Fatalf("expected page-limit warnings to be aggregated, got %d warnings: %+v", got, result.Warnings)
	}
}

func TestCrawlNavigationPathUsesURLDirectories(t *testing.T) {
	prefix, err := url.Parse("https://xiaolinnote.com/ai/")
	if err != nil {
		t.Fatal(err)
	}

	tests := []struct {
		name  string
		raw   string
		title string
		want  []string
	}{
		{
			name:  "prefix root",
			raw:   "https://xiaolinnote.com/ai/",
			title: "大模型面试题",
			want:  []string{"大模型面试题"},
		},
		{
			name:  "directory chapter",
			raw:   "https://xiaolinnote.com/ai/rag/1_whatisrag.html",
			title: "1. 什么是 RAG？",
			want:  []string{"RAG", "1. 什么是 RAG？"},
		},
		{
			name:  "directory index page",
			raw:   "https://xiaolinnote.com/ai/agent/",
			title: "Agent 面试题介绍",
			want:  []string{"Agent", "Agent 面试题介绍"},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			page, err := url.Parse(tt.raw)
			if err != nil {
				t.Fatal(err)
			}
			if got := crawlNavigationPath(page, prefix, tt.title); !reflect.DeepEqual(got, tt.want) {
				t.Fatalf("crawlNavigationPath() = %#v, want %#v", got, tt.want)
			}
		})
	}
}

func warningCount(warnings []ConversionWarning, code string) int {
	count := 0
	for _, warning := range warnings {
		if warning.Code == code {
			count++
		}
	}
	return count
}

func TestCrossChapterRewriteUsesBasename(t *testing.T) {
	page1 := `<!doctype html>
<html lang="en">
  <body>
    <article>
      <h1>Chapter One</h1>
      <p>First page content.</p>
      <a href="chapter-two.html">Go to chapter two</a>
      <a href="chapter-two.html#section-b">Go to section B</a>
    </article>
  </body>
</html>`
	page2 := `<!doctype html>
<html lang="en">
  <body>
    <article>
      <h1>Chapter Two</h1>
      <p id="section-b">Second page content.</p>
      <a href="chapter-one.html">Back to chapter one</a>
    </article>
  </body>
</html>`

	result, err := ConvertCrawl(CrawlInput{
		StartURL: "https://example.com/book/chapter-one.html",
		Pages: []CrawlPage{
			{URL: "https://example.com/book/chapter-one.html", HTML: &page1},
			{URL: "https://example.com/book/chapter-two.html", HTML: &page2},
		},
		Metadata: BookMetadata{Title: "Cross Chapter", Language: "en"},
		Options:  ConversionOptions{IncludeImages: false},
		Crawl: CrawlOptions{
			PrefixURL:         "https://example.com/book/",
			MaxDepth:          2,
			MaxPages:          10,
			MaxTotalBytes:     1024 * 1024,
			MaxDurationMillis: 0,
		},
	})
	if err != nil {
		t.Fatalf("ConvertCrawl returned error: %v", err)
	}
	if result.ChapterCount != 2 {
		t.Fatalf("expected 2 chapters, got %d", result.ChapterCount)
	}

	// Body XHTML inside the EPUB must use sibling basenames.
	chapter1 := epubEntry(t, result.EPUBBytes, "EPUB/chapters/chapter-1.xhtml")
	if !strings.Contains(chapter1, `href="chapter-2.xhtml"`) {
		t.Fatalf("chapter body missing basename cross-chapter href; xhtml:\n%s", chapter1)
	}
	if strings.Contains(chapter1, `href="chapters/chapter-2.xhtml"`) {
		t.Fatalf("chapter body must not use OPF-relative chapters/ path; xhtml:\n%s", chapter1)
	}
	if !strings.Contains(chapter1, `href="chapter-2.xhtml#section-b"`) {
		t.Fatalf("chapter body missing basename+fragment href; xhtml:\n%s", chapter1)
	}

	// OPF/nav still use full chapterHref paths.
	opf := epubEntry(t, result.EPUBBytes, "EPUB/package.opf")
	if !strings.Contains(opf, `href="chapters/chapter-2.xhtml"`) {
		t.Fatalf("OPF should keep chapters/ prefix; opf:\n%s", opf)
	}
	nav := epubEntry(t, result.EPUBBytes, "EPUB/nav.xhtml")
	if !strings.Contains(nav, `href="chapters/chapter-2.xhtml"`) {
		t.Fatalf("nav should keep chapters/ prefix; nav:\n%s", nav)
	}
}

func TestMaxDurationMillisZeroDoesNotStopCrawl(t *testing.T) {
	page1 := `<!doctype html>
<html lang="en">
  <body>
    <article>
      <h1>Start</h1>
      <p>Readable crawl start content.</p>
      <a href="two.html">Two</a>
    </article>
  </body>
</html>`
	page2 := `<!doctype html>
<html lang="en">
  <body>
    <article>
      <h1>Two</h1>
      <p>Second page readable content.</p>
    </article>
  </body>
</html>`

	result, err := ConvertCrawl(CrawlInput{
		StartURL: "https://example.com/book/index.html",
		Pages: []CrawlPage{
			{URL: "https://example.com/book/index.html", HTML: &page1},
			{URL: "https://example.com/book/two.html", HTML: &page2},
		},
		Metadata: BookMetadata{Title: "Unlimited Duration", Language: "en"},
		Options:  ConversionOptions{IncludeImages: false},
		Crawl: CrawlOptions{
			PrefixURL:         "https://example.com/book/",
			MaxDepth:          2,
			MaxPages:          10,
			MaxTotalBytes:     1024 * 1024,
			MaxDurationMillis: 0, // unlimited
		},
	})
	if err != nil {
		t.Fatalf("ConvertCrawl returned error: %v", err)
	}
	if result.ChapterCount < 2 {
		t.Fatalf("expected multi-page crawl with MaxDurationMillis=0, got %d chapters; warnings=%+v", result.ChapterCount, result.Warnings)
	}
	if got := warningCount(result.Warnings, "crawl_time_limit"); got != 0 {
		t.Fatalf("expected no crawl_time_limit warning for MaxDurationMillis=0, got %d: %+v", got, result.Warnings)
	}
}

func TestMakeSafeHrefRelativeSameDocumentFragment(t *testing.T) {
	source, err := url.Parse("https://example.com/docs/page.html")
	if err != nil {
		t.Fatal(err)
	}
	ids := map[string]bool{"intro": true, "section-2": true}

	tests := []struct {
		name string
		href string
		want string
	}{
		{name: "fragment only", href: "#intro", want: "#intro"},
		{name: "relative same-doc with fragment", href: "page.html#section-2", want: "#section-2"},
		{name: "dot-relative same-doc fragment", href: "./page.html#intro", want: "#intro"},
		{name: "absolute same-doc fragment", href: "https://example.com/docs/page.html#intro", want: "#intro"},
		{name: "absolute external kept", href: "https://other.example.com/x", want: "https://other.example.com/x"},
		{name: "mailto kept", href: "mailto:a@b.com", want: "mailto:a@b.com"},
		{name: "javascript dropped", href: "javascript:alert(1)", want: ""},
		{name: "unknown fragment dropped", href: "#missing", want: ""},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := makeSafeHref(tt.href, source, ids, nil)
			if got != tt.want {
				t.Fatalf("makeSafeHref(%q) = %q, want %q", tt.href, got, tt.want)
			}
		})
	}
}

func epubEntry(t *testing.T, epub []byte, name string) string {
	t.Helper()
	zr, err := zip.NewReader(bytes.NewReader(epub), int64(len(epub)))
	if err != nil {
		t.Fatalf("open epub zip: %v", err)
	}
	for _, f := range zr.File {
		if f.Name != name {
			continue
		}
		rc, err := f.Open()
		if err != nil {
			t.Fatalf("open %s: %v", name, err)
		}
		defer rc.Close()
		data, err := io.ReadAll(rc)
		if err != nil {
			t.Fatalf("read %s: %v", name, err)
		}
		return string(data)
	}
	t.Fatalf("epub entry %q not found", name)
	return ""
}
