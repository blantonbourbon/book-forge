package converter

import "encoding/json"

type ConversionMode string

const (
	ModeSingle ConversionMode = "single"
	ModeCrawl  ConversionMode = "crawl"
)

type BookMetadata struct {
	Title       string `json:"title"`
	Author      string `json:"author"`
	Language    string `json:"language"`
	Description string `json:"description"`
}

type ConversionOptions struct {
	IncludeImages bool `json:"includeImages"`
}

type CrawlOptions struct {
	PrefixURL         string `json:"prefixUrl"`
	MaxDepth          int    `json:"maxDepth"`
	MaxPages          int    `json:"maxPages"`
	MaxTotalBytes     int    `json:"maxTotalBytes"`
	MaxDurationMillis int64  `json:"maxDurationMillis"`
}

func DefaultCrawlOptions() CrawlOptions {
	return CrawlOptions{
		MaxDepth:          3,
		MaxPages:          50,
		MaxTotalBytes:     10 * 1024 * 1024,
		MaxDurationMillis: 30000,
	}
}

type SinglePageInput struct {
	SourceURL string            `json:"sourceUrl"`
	HTML      string            `json:"html"`
	Resources []CrawlResource   `json:"resources,omitempty"`
	Metadata  BookMetadata      `json:"metadata"`
	Options   ConversionOptions `json:"options"`
}

type CrawlPage struct {
	URL     string  `json:"url"`
	HTML    *string `json:"html,omitempty"`
	Failure *string `json:"failure,omitempty"`
}

type CrawlResource struct {
	URL       string  `json:"url"`
	MediaType string  `json:"mediaType"`
	Bytes     []byte  `json:"bytes"`
	Failure   *string `json:"failure,omitempty"`
}

const CrawlTimeLimitFailure = "crawl_time_limit"

type CrawlInput struct {
	StartURL  string            `json:"startUrl"`
	Pages     []CrawlPage       `json:"pages"`
	Resources []CrawlResource   `json:"resources"`
	Metadata  BookMetadata      `json:"metadata"`
	Options   ConversionOptions `json:"options"`
	Crawl     CrawlOptions      `json:"crawl"`
}

type ConversionResult struct {
	EPUBBytes        []byte              `json:"-"`
	DownloadFilename string              `json:"downloadFilename"`
	ChapterCount     int                 `json:"chapterCount"`
	Metadata         SanitizedMetadata   `json:"metadata"`
	Warnings         []ConversionWarning `json:"warnings"`
}

func (r ConversionResult) MarshalJSON() ([]byte, error) {
	type alias ConversionResult
	return json.Marshal(&struct {
		EPUBSize int `json:"epubSize"`
		*alias
	}{
		EPUBSize: len(r.EPUBBytes),
		alias:    (*alias)(&r),
	})
}

type ConversionWarning struct {
	Code     string  `json:"code"`
	Message  string  `json:"message"`
	Affected *string `json:"affected,omitempty"`
}

type ConversionError struct {
	Code    string
	Message string
}

func (e *ConversionError) Error() string {
	return e.Message
}

func NewInvalidSourceURLError(message string) *ConversionError {
	return &ConversionError{Code: "invalid_source_url", Message: message}
}

func NewNoReadableContentError() *ConversionError {
	return &ConversionError{Code: "no_readable_content", Message: "The page did not contain readable content after sanitization."}
}

func NewEpubGenerationError(message string) *ConversionError {
	return &ConversionError{Code: "epub_generation_failed", Message: "The EPUB could not be generated from the supplied content."}
}

func BoundaryName() string {
	return "converter"
}

func ValidateSourceURL(sourceURL string) (string, error) {
	u, err := parseURL(sourceURL)
	if err != nil {
		return "", NewInvalidSourceURLError("Source URL must be an absolute HTTP or HTTPS URL.")
	}
	scheme := u.Scheme
	if scheme != "http" && scheme != "https" {
		return "", NewInvalidSourceURLError("Source URL must use HTTP or HTTPS.")
	}
	return sourceURL, nil
}
