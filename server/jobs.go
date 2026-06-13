package server

import (
	"net/url"
	"strings"
	"sync"
	"time"

	"github.com/google/uuid"

	"github.com/blantonbourbon/book-forge/converter"
)

const (
	defaultFetchTimeoutMs = 30000
	defaultMaxTotalBytes  = 10 * 1024 * 1024
	maxMetadataChars      = 512
	maxCrawlDepth         = 10
	maxCrawlPages         = 100
	maxTotalBytes         = 20 * 1024 * 1024
	maxDurationMillis     = 180000
	jobRetention          = 1 * time.Hour
	jobSweepInterval      = 1 * time.Minute
)

type AppState struct {
	Jobs           *JobManager
	Fetcher        *SharedFetcher
	BrowserFetcher *BrowserFetcher
	StaticRoot     string
}

func NewAppState(fetcher *SharedFetcher) *AppState {
	return &AppState{
		Jobs:    NewJobManager(),
		Fetcher: fetcher,
	}
}

type JobManager struct {
	mu   sync.RWMutex
	jobs map[uuid.UUID]*JobRecord
}

func NewJobManager() *JobManager {
	m := &JobManager{
		jobs: make(map[uuid.UUID]*JobRecord),
	}
	go m.sweepLoop()
	return m
}

func (m *JobManager) sweepLoop() {
	ticker := time.NewTicker(jobSweepInterval)
	defer ticker.Stop()
	for range ticker.C {
		m.sweep(time.Now())
	}
}

func (m *JobManager) sweep(now time.Time) {
	cutoff := now.Add(-jobRetention)
	m.mu.Lock()
	defer m.mu.Unlock()
	for id, rec := range m.jobs {
		if rec.Status != StatusCompleted && rec.Status != StatusFailed {
			continue
		}
		if rec.CompletedAt.IsZero() || rec.CompletedAt.After(cutoff) {
			continue
		}
		delete(m.jobs, id)
	}
}

type CreateJobRequest struct {
	SourceURL *string          `json:"sourceUrl"`
	Mode      *string          `json:"mode"`
	Metadata  *APIMetadata     `json:"metadata,omitempty"`
	Options   APIOptions       `json:"options"`
	Crawl     *APICrawlOptions `json:"crawl,omitempty"`
}

type APIMetadata struct {
	Title       *string `json:"title,omitempty"`
	Author      *string `json:"author,omitempty"`
	Language    *string `json:"language,omitempty"`
	Description *string `json:"description,omitempty"`
}

type APIOptions struct {
	IncludeImages bool   `json:"includeImages"`
	OutputTarget  string `json:"outputTarget"`
	UseBrowser    bool   `json:"useBrowser"`
}

type APICrawlOptions struct {
	PrefixURL         *string `json:"prefixUrl,omitempty"`
	MaxDepth          *int    `json:"maxDepth,omitempty"`
	MaxPages          *int    `json:"maxPages,omitempty"`
	MaxTotalBytes     *int    `json:"maxTotalBytes,omitempty"`
	MaxDurationMillis *int64  `json:"maxDurationMillis,omitempty"`
}

type JobMode string

const (
	ModeSingle JobMode = "single"
	ModeCrawl  JobMode = "crawl"
)

type JobStatus string

const (
	StatusQueued    JobStatus = "queued"
	StatusRunning   JobStatus = "running"
	StatusCompleted JobStatus = "completed"
	StatusFailed    JobStatus = "failed"
)

type JobResponse struct {
	ID          string                        `json:"id"`
	Status      JobStatus                     `json:"status"`
	Mode        JobMode                       `json:"mode"`
	Summary     JobSummary                    `json:"summary"`
	Progress    JobProgress                   `json:"progress"`
	Warnings    []converter.ConversionWarning `json:"warnings"`
	Errors      []ErrorBody                   `json:"errors"`
	DownloadURL *string                       `json:"downloadUrl,omitempty"`
}

type JobSummary struct {
	SourceURL string                 `json:"sourceUrl"`
	Mode      JobMode                `json:"mode"`
	Metadata  converter.BookMetadata `json:"metadata"`
	Options   APIOptions             `json:"options"`
	Crawl     *CrawlSummary          `json:"crawl,omitempty"`
}

type CrawlSummary struct {
	PrefixURL         string `json:"prefixUrl"`
	MaxDepth          int    `json:"maxDepth"`
	MaxPages          int    `json:"maxPages"`
	MaxTotalBytes     int    `json:"maxTotalBytes"`
	MaxDurationMillis int64  `json:"maxDurationMillis"`
}

func (c CrawlSummary) ToCrawlOptions() converter.CrawlOptions {
	return converter.CrawlOptions{
		PrefixURL:         c.PrefixURL,
		MaxDepth:          c.MaxDepth,
		MaxPages:          c.MaxPages,
		MaxTotalBytes:     c.MaxTotalBytes,
		MaxDurationMillis: c.MaxDurationMillis,
	}
}

type JobProgress struct {
	Percent         int `json:"percent"`
	PagesDiscovered int `json:"pagesDiscovered"`
	PagesFetched    int `json:"pagesFetched"`
	PagesSkipped    int `json:"pagesSkipped"`
	CurrentDepth    int `json:"currentDepth"`
	BytesFetched    int `json:"bytesFetched"`
	MaxPages        int `json:"maxPages"`
	MaxDepth        int `json:"maxDepth"`
	MaxTotalBytes   int `json:"maxTotalBytes"`
}

type Artifact struct {
	Filename string
	Bytes    []byte
}

type JobRecord struct {
	ID          uuid.UUID
	Status      JobStatus
	Summary     JobSummary
	Progress    JobProgress
	Warnings    []converter.ConversionWarning
	Errors      []ErrorBody
	Artifact    *Artifact
	CompletedAt time.Time
}

func queuedProgress(summary *JobSummary) JobProgress {
	p := JobProgress{}
	if summary.Crawl != nil {
		p.MaxPages = summary.Crawl.MaxPages
		p.MaxDepth = summary.Crawl.MaxDepth
		p.MaxTotalBytes = summary.Crawl.MaxTotalBytes
	} else {
		p.MaxPages = 1
		p.MaxTotalBytes = defaultMaxTotalBytes
	}
	return p
}

func runningProgress(summary *JobSummary) JobProgress {
	p := queuedProgress(summary)
	p.Percent = 5
	return p
}

func (p JobProgress) Completed() JobProgress {
	p.Percent = 100
	return p
}

func (p JobProgress) Failed() JobProgress {
	if p.Percent > 99 {
		p.Percent = 99
	}
	return p
}

func (r *JobRecord) Response() JobResponse {
	resp := JobResponse{
		ID:       r.ID.String(),
		Status:   r.Status,
		Mode:     r.Summary.Mode,
		Summary:  r.Summary,
		Progress: r.Progress,
		Warnings: r.Warnings,
		Errors:   r.Errors,
	}
	if r.Status == StatusCompleted {
		url := "/api/jobs/" + r.ID.String() + "/download"
		resp.DownloadURL = &url
	}
	return resp
}

func (m *JobManager) CreateJob(fetcher *SharedFetcher, browserFetcher *BrowserFetcher, summary JobSummary) (*JobResponse, error) {
	id := uuid.New()
	record := &JobRecord{
		ID:       id,
		Status:   StatusQueued,
		Summary:  summary,
		Progress: queuedProgress(&summary),
	}
	m.mu.Lock()
	m.jobs[id] = record
	m.mu.Unlock()

	go m.executeJob(id, fetcher, browserFetcher, summary)

	return m.GetResponse(id)
}

func (m *JobManager) GetResponse(id uuid.UUID) (*JobResponse, error) {
	m.mu.RLock()
	defer m.mu.RUnlock()
	record, ok := m.jobs[id]
	if !ok {
		return nil, &APIError{Status: 404, Body: ErrorBody{Code: "job_not_found", Message: "The requested job was not found."}}
	}
	resp := record.Response()
	return &resp, nil
}

func (m *JobManager) Artifact(id uuid.UUID) (JobStatus, *Artifact) {
	m.mu.RLock()
	defer m.mu.RUnlock()
	record, ok := m.jobs[id]
	if !ok {
		return "", nil
	}
	return record.Status, record.Artifact
}

func (m *JobManager) executeJob(id uuid.UUID, fetcher *SharedFetcher, browserFetcher *BrowserFetcher, summary JobSummary) {
	m.markRunning(id)

	var result *converter.ConversionResult
	var progress JobProgress
	var err error

	switch summary.Mode {
	case ModeSingle:
		result, progress, err = executeSingle(fetcher, browserFetcher, summary)
	case ModeCrawl:
		result, progress, err = executeCrawl(id, m, fetcher, browserFetcher, summary)
	}

	if err != nil {
		errBody := ErrorBody{
			Code:    "conversion_failed",
			Message: sanitizeMessage(err.Error()),
		}
		m.markFailed(id, errBody, progress)
		return
	}

	m.markCompleted(id, result, progress)
}

func (m *JobManager) markRunning(id uuid.UUID) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if record, ok := m.jobs[id]; ok {
		record.Status = StatusRunning
		record.Progress = runningProgress(&record.Summary)
	}
}

func (m *JobManager) UpdateProgress(id uuid.UUID, progress JobProgress) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if record, ok := m.jobs[id]; ok && record.Status == StatusRunning {
		record.Progress = progress
	}
}

func (m *JobManager) markCompleted(id uuid.UUID, result *converter.ConversionResult, progress JobProgress) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if record, ok := m.jobs[id]; ok {
		record.Status = StatusCompleted
		record.Progress = progress.Completed()
		record.Warnings = result.Warnings
		record.Errors = nil
		record.Artifact = &Artifact{
			Filename: result.DownloadFilename,
			Bytes:    result.EPUBBytes,
		}
		record.CompletedAt = time.Now()
	}
}

func (m *JobManager) markFailed(id uuid.UUID, errBody ErrorBody, progress JobProgress) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if record, ok := m.jobs[id]; ok {
		record.Status = StatusFailed
		record.Progress = progress.Failed()
		record.Errors = []ErrorBody{errBody}
		record.Artifact = nil
		record.CompletedAt = time.Now()
	}
}

func ValidateCreateRequest(req CreateJobRequest) (*JobSummary, error) {
	var fields []string

	if req.SourceURL == nil || strings.TrimSpace(*req.SourceURL) == "" {
		fields = append(fields, "sourceUrl")
		return nil, ValidationError("A source URL is required.", fields)
	}
	sourceURL := strings.TrimSpace(*req.SourceURL)

	parsedSource, err := url.Parse(sourceURL)
	if err != nil || (parsedSource.Scheme != "http" && parsedSource.Scheme != "https") {
		fields = append(fields, "sourceUrl")
	}

	if req.Mode == nil {
		fields = append(fields, "mode")
		return nil, ValidationError("A conversion mode is required.", fields)
	}
	rawMode := strings.TrimSpace(*req.Mode)
	var mode JobMode
	switch rawMode {
	case "single":
		mode = ModeSingle
	case "crawl":
		mode = ModeCrawl
	default:
		fields = append(fields, "mode")
		mode = ModeSingle
	}

	metadata := metadataFromRequest(req.Metadata, &fields)

	var crawl *CrawlSummary
	if mode == ModeCrawl {
		if parsedSource == nil {
			return nil, ValidationError("Source URL must be absolute HTTP or HTTPS.", fields)
		}
		crawl = crawlFromRequest(req.Crawl, parsedSource, &fields)
	}

	if len(fields) > 0 {
		return nil, ValidationError("One or more job request fields were invalid.", fields)
	}

	return &JobSummary{
		SourceURL: sourceURL,
		Mode:      mode,
		Metadata:  metadata,
		Options:   req.Options,
		Crawl:     crawl,
	}, nil
}

func EnforceCreateRequestSecurity(summary *JobSummary) error {
	if err := ValidateNetworkURL(summary.SourceURL); err != nil {
		return ValidationError(err.Error(), []string{"sourceUrl"})
	}
	if summary.Crawl != nil {
		if err := ValidateNetworkURL(summary.Crawl.PrefixURL); err != nil {
			return ValidationError(err.Error(), []string{"crawl.prefixUrl"})
		}
	}
	return nil
}

func metadataFromRequest(metadata *APIMetadata, fields *[]string) converter.BookMetadata {
	result := converter.BookMetadata{
		Title:    "Untitled Book",
		Author:   "Unknown Author",
		Language: "en",
	}
	if metadata != nil {
		if metadata.Title != nil {
			result.Title = strings.TrimSpace(*metadata.Title)
		}
		if metadata.Author != nil {
			result.Author = strings.TrimSpace(*metadata.Author)
		}
		if metadata.Language != nil {
			result.Language = strings.TrimSpace(*metadata.Language)
		}
		if metadata.Description != nil {
			result.Description = strings.TrimSpace(*metadata.Description)
		}
	}

	checks := []struct {
		field string
		value string
	}{
		{"metadata.title", result.Title},
		{"metadata.author", result.Author},
		{"metadata.language", result.Language},
		{"metadata.description", result.Description},
	}
	for _, c := range checks {
		if len([]rune(c.value)) > maxMetadataChars || containsForbiddenControl(c.value) {
			*fields = append(*fields, c.field)
		}
	}

	if !validLanguageTag(result.Language) {
		*fields = append(*fields, "metadata.language")
	}

	return result
}

func crawlFromRequest(crawl *APICrawlOptions, sourceURL *url.URL, fields *[]string) *CrawlSummary {
	prefixURL := defaultPrefix(sourceURL)
	if crawl != nil && crawl.PrefixURL != nil && strings.TrimSpace(*crawl.PrefixURL) != "" {
		prefixURL = strings.TrimSpace(*crawl.PrefixURL)
	}

	if parsed, err := url.Parse(prefixURL); err != nil || (parsed.Scheme != "http" && parsed.Scheme != "https") {
		*fields = append(*fields, "crawl.prefixUrl")
	}

	depth := 3
	if crawl != nil && crawl.MaxDepth != nil {
		depth = *crawl.MaxDepth
	}
	pages := 50
	if crawl != nil && crawl.MaxPages != nil {
		pages = *crawl.MaxPages
	}
	totalBytes := defaultMaxTotalBytes
	if crawl != nil && crawl.MaxTotalBytes != nil {
		totalBytes = *crawl.MaxTotalBytes
	}
	durationMillis := int64(defaultFetchTimeoutMs)
	if crawl != nil && crawl.MaxDurationMillis != nil {
		durationMillis = *crawl.MaxDurationMillis
	}

	if depth > maxCrawlDepth {
		*fields = append(*fields, "crawl.maxDepth")
	}
	if pages == 0 || pages > maxCrawlPages {
		*fields = append(*fields, "crawl.maxPages")
	}
	if totalBytes == 0 || totalBytes > maxTotalBytes {
		*fields = append(*fields, "crawl.maxTotalBytes")
	}
	if durationMillis <= 0 || durationMillis > int64(maxDurationMillis) {
		*fields = append(*fields, "crawl.maxDurationMillis")
	}

	return &CrawlSummary{
		PrefixURL:         prefixURL,
		MaxDepth:          depth,
		MaxPages:          pages,
		MaxTotalBytes:     totalBytes,
		MaxDurationMillis: durationMillis,
	}
}

func defaultPrefix(sourceURL *url.URL) string {
	return DefaultPrefixURL(sourceURL)
}

func executeSingle(fetcher *SharedFetcher, browserFetcher *BrowserFetcher, summary JobSummary) (*converter.ConversionResult, JobProgress, error) {
	progress := runningProgress(&summary)
	sourceURL, _ := url.Parse(summary.SourceURL)

	var fetched *FetchedResponse
	var err error
	if summary.Options.UseBrowser && browserFetcher == nil {
		return nil, progress, NewFetchError("browser_unavailable", "Browser rendering is not configured on this server.")
	}
	if summary.Options.UseBrowser {
		fetched, err = fetchHTMLBrowser(browserFetcher, sourceURL.String(), time.Duration(defaultFetchTimeoutMs)*time.Millisecond, defaultMaxTotalBytes)
	} else {
		fetched, err = fetchHTML(fetcher, sourceURL.String(), time.Duration(defaultFetchTimeoutMs)*time.Millisecond, defaultMaxTotalBytes)
	}
	if err != nil {
		return nil, progress, err
	}

	progress.PagesDiscovered = 1
	progress.PagesFetched = 1
	progress.BytesFetched = len(fetched.Bytes)
	progress.Percent = 70

	baseURL, _ := url.Parse(fetched.FinalURL)
	if baseURL == nil {
		baseURL = sourceURL
	}

	html, _ := fetched.Text()

	var resources []converter.CrawlResource
	if summary.Options.IncludeImages {
		imageURLs := ExtractImageURLs(html, baseURL)
		seen := make(map[string]bool)
		for _, imgURL := range imageURLs {
			if imgURL.Scheme != "http" && imgURL.Scheme != "https" {
				continue
			}
			key := imgURL.String()
			if seen[key] {
				continue
			}
			seen[key] = true

			res, err := fetcher.Fetch(imgURL.String(), time.Duration(defaultFetchTimeoutMs)*time.Millisecond, defaultMaxTotalBytes)
			if err != nil {
				resources = append(resources, converter.CrawlResource{
					URL:       imgURL.String(),
					MediaType: "application/octet-stream",
					Failure:   strPtr(err.Error()),
				})
				continue
			}
			progress.BytesFetched += len(res.Bytes)
			resources = append(resources, converter.CrawlResource{
				URL:       imgURL.String(),
				MediaType: res.MediaType,
				Bytes:     res.Bytes,
			})
		}
	}

	input := converter.SinglePageInput{
		SourceURL: fetched.FinalURL,
		HTML:      html,
		Resources: resources,
		Metadata:  summary.Metadata,
		Options: converter.ConversionOptions{
			IncludeImages: summary.Options.IncludeImages,
		},
	}

	result, err := converter.ConvertSinglePage(input)
	if err != nil {
		return nil, progress, err
	}

	return result, progress, nil
}

func executeCrawl(id uuid.UUID, jobs *JobManager, fetcher *SharedFetcher, browserFetcher *BrowserFetcher, summary JobSummary) (*converter.ConversionResult, JobProgress, error) {
	crawl := summary.Crawl
	progress := runningProgress(&summary)
	sourceURL, _ := url.Parse(summary.SourceURL)

	var pages []converter.CrawlPage
	var resources []converter.CrawlResource
	started := time.Now()
	deadline := started.Add(time.Duration(crawl.MaxDurationMillis) * time.Millisecond)

	type queueEntry struct {
		url   *url.URL
		depth int
	}
	queue := []queueEntry{{url: sourceURL, depth: 0}}
	seenPages := map[string]bool{normalizePageKey(sourceURL): true}
	seenResources := make(map[string]bool)
	crawlTimeLimitReached := false

	progress.PagesDiscovered = 1
	progress.Percent = 10
	jobs.UpdateProgress(id, progress)

	for len(queue) > 0 {
		entry := queue[0]
		queue = queue[1:]
		pageURL := entry.url
		depth := entry.depth

		remaining := deadline.Sub(time.Now())
		if remaining <= 0 {
			crawlTimeLimitReached = true
			break
		}

		if depth > progress.CurrentDepth {
			progress.CurrentDepth = depth
		}

		remainingBytes := crawl.MaxTotalBytes - progress.BytesFetched
		if remainingBytes <= 0 {
			pages = append(pages, converter.CrawlPage{
				URL:     pageURL.String(),
				Failure: strPtr(converter.CrawlByteLimitFailure),
			})
			progress.PagesSkipped++
			continue
		}

		var pageFetchErr error
		var fetched *FetchedResponse
		if summary.Options.UseBrowser && browserFetcher == nil {
			pageFetchErr = NewFetchError("browser_unavailable", "Browser rendering is not configured on this server.")
		} else if summary.Options.UseBrowser {
			fetched, pageFetchErr = fetchHTMLBrowser(browserFetcher, pageURL.String(), remaining, remainingBytes)
		} else {
			fetched, pageFetchErr = fetchHTML(fetcher, pageURL.String(), remaining, remainingBytes)
		}
		if pageFetchErr != nil {
			if depth == 0 {
				return nil, progress, pageFetchErr
			}
			progress.PagesSkipped++
			pages = append(pages, converter.CrawlPage{
				URL:     pageURL.String(),
				Failure: strPtr(crawlFailureForFetchError(pageFetchErr, remainingBytes, crawl.MaxTotalBytes)),
			})
			if time.Now().After(deadline) {
				crawlTimeLimitReached = true
				break
			}
			continue
		}

		if time.Now().After(deadline) {
			crawlTimeLimitReached = true
		}

		pageBytes := len(fetched.Bytes)
		if progress.BytesFetched+pageBytes > crawl.MaxTotalBytes {
			pages = append(pages, converter.CrawlPage{
				URL:     pageURL.String(),
				Failure: strPtr(converter.CrawlByteLimitFailure),
			})
			progress.PagesSkipped++
			continue
		}

		html, _ := fetched.Text()
		progress.BytesFetched += pageBytes
		progress.PagesFetched++
		progress.Percent = 30 + (progress.PagesFetched*50)/max(crawl.MaxPages, 1)

		baseURL, err := url.Parse(fetched.FinalURL)
		if err != nil || baseURL == nil {
			baseURL = pageURL
		}

		pages = append(pages, converter.CrawlPage{
			URL:  fetched.FinalURL,
			HTML: &html,
		})

		if summary.Options.IncludeImages {
			imageURLs := ExtractImageURLs(html, baseURL)
			for i, imgURL := range imageURLs {
				if imgURL.Scheme != "http" && imgURL.Scheme != "https" {
					continue
				}
				key := imgURL.String()
				if seenResources[key] {
					continue
				}
				seenResources[key] = true

				remainingBytes := crawl.MaxTotalBytes - progress.BytesFetched
				if remainingBytes <= 0 {
					resources = append(resources, converter.CrawlResource{
						URL:       imgURL.String(),
						MediaType: "application/octet-stream",
						Failure:   strPtr(converter.CrawlByteLimitFailure),
					})
					continue
				}

				res, err := fetcher.Fetch(imgURL.String(), deadline.Sub(time.Now()), remainingBytes)
				if err != nil {
					resources = append(resources, converter.CrawlResource{
						URL:       imgURL.String(),
						MediaType: "application/octet-stream",
						Failure:   strPtr(crawlFailureForFetchError(err, remainingBytes, crawl.MaxTotalBytes)),
					})
				} else if progress.BytesFetched+len(res.Bytes) > crawl.MaxTotalBytes {
					resources = append(resources, converter.CrawlResource{
						URL:       imgURL.String(),
						MediaType: "application/octet-stream",
						Failure:   strPtr(converter.CrawlByteLimitFailure),
					})
				} else {
					progress.BytesFetched += len(res.Bytes)
					resources = append(resources, converter.CrawlResource{
						URL:       imgURL.String(),
						MediaType: res.MediaType,
						Bytes:     res.Bytes,
					})
				}
				if time.Now().After(deadline) {
					crawlTimeLimitReached = true
					for _, remaining := range imageURLs[i+1:] {
						if remaining.Scheme != "http" && remaining.Scheme != "https" {
							continue
						}
						rkey := remaining.String()
						if seenResources[rkey] {
							continue
						}
						seenResources[rkey] = true
						resources = append(resources, converter.CrawlResource{
							URL:       rkey,
							MediaType: "application/octet-stream",
							Failure:   strPtr(converter.CrawlTimeLimitFailure),
						})
					}
					break
				}
			}
		}

		if time.Now().After(deadline) {
			crawlTimeLimitReached = true
			jobs.UpdateProgress(id, progress)
			break
		}

		for _, linkURL := range ExtractLinkURLs(html, baseURL) {
			if !isInCrawlScopeServer(linkURL, sourceURL, crawl.PrefixURL) {
				continue
			}
			key := normalizePageKey(linkURL)
			if seenPages[key] {
				continue
			}
			if depth+1 > crawl.MaxDepth || len(seenPages) >= crawl.MaxPages {
				continue
			}
			seenPages[key] = true
			progress.PagesDiscovered = len(seenPages)
			queue = append(queue, queueEntry{url: withoutFragment(linkURL), depth: depth + 1})
		}

		jobs.UpdateProgress(id, progress)
	}

	input := converter.CrawlInput{
		StartURL:  summary.SourceURL,
		Pages:     pages,
		Resources: resources,
		Metadata:  summary.Metadata,
		Options: converter.ConversionOptions{
			IncludeImages: summary.Options.IncludeImages,
		},
		Crawl: crawl.ToCrawlOptions(),
	}

	result, err := converter.ConvertCrawl(input)
	if err != nil {
		return nil, progress, err
	}

	if crawlTimeLimitReached {
		hasTimeLimit := false
		for _, w := range result.Warnings {
			if w.Code == "crawl_time_limit" {
				hasTimeLimit = true
				break
			}
		}
		if !hasTimeLimit {
			result.Warnings = append(result.Warnings, converter.ConversionWarning{
				Code:    "crawl_time_limit",
				Message: "Crawl stopped because the configured time limit was reached.",
			})
		}
	}

	return result, progress, nil
}

func crawlFailureForFetchError(err error, remainingBytes, maxTotalBytes int) string {
	if fetchErr, ok := err.(*FetchError); ok && fetchErr.Code == "response_too_large" && remainingBytes < maxTotalBytes {
		return converter.CrawlByteLimitFailure
	}
	return err.Error()
}

func fetchHTML(fetcher *SharedFetcher, urlStr string, timeout time.Duration, maxBytes int) (*FetchedResponse, error) {
	fetched, err := fetcher.Fetch(urlStr, timeout, maxBytes)
	if err != nil {
		return nil, err
	}
	if !IsHTMLike(fetched.MediaType) {
		return nil, NewFetchError("unsupported_media_type", "Fetched content was not an HTML document.")
	}
	return fetched, nil
}

func fetchHTMLBrowser(fetcher *BrowserFetcher, urlStr string, timeout time.Duration, maxBytes int) (*FetchedResponse, error) {
	fetched, err := fetcher.Fetch(urlStr, timeout, maxBytes)
	if err != nil {
		return nil, err
	}
	if !IsHTMLike(fetched.MediaType) {
		return nil, NewFetchError("unsupported_media_type", "Fetched content was not an HTML document.")
	}
	return fetched, nil
}

func isInCrawlScopeServer(candidate, startURL *url.URL, prefixURLStr string) bool {
	if candidate.Scheme != "http" && candidate.Scheme != "https" {
		return false
	}
	if candidate.Scheme != startURL.Scheme || !strings.EqualFold(candidate.Hostname(), startURL.Hostname()) {
		return false
	}
	candStr := withoutFragment(candidate).String()
	prefixURL, _ := url.Parse(prefixURLStr)
	prefixStr := withoutFragment(prefixURL).String()
	return strings.HasPrefix(candStr, prefixStr)
}

func normalizePageKey(u *url.URL) string {
	nu := *u
	nu.Fragment = ""
	nu.RawQuery = ""
	return nu.String()
}

func containsForbiddenControl(s string) bool {
	for _, ch := range s {
		if ch < 32 && ch != '\n' && ch != '\r' && ch != '\t' {
			return true
		}
	}
	return false
}

func validLanguageTag(language string) bool {
	parts := strings.Split(language, "-")
	if len(parts) == 0 {
		return false
	}
	primary := parts[0]
	if len(primary) < 2 || len(primary) > 8 {
		return false
	}
	for _, ch := range primary {
		if !((ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z')) {
			return false
		}
	}
	for _, part := range parts[1:] {
		if len(part) < 1 || len(part) > 8 {
			return false
		}
		for _, ch := range part {
			if !((ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || (ch >= '0' && ch <= '9')) {
				return false
			}
		}
	}
	return true
}

func strPtr(s string) *string {
	return &s
}

func withoutFragment(u *url.URL) *url.URL {
	nu := *u
	nu.Fragment = ""
	return &nu
}
