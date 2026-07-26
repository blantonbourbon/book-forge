package converter

import (
	"bytes"
	"fmt"
	"net/url"
	"regexp"
	"strings"
	"time"
)

type pageSource struct {
	html    *string
	failure *string
}

type resourceSource struct {
	mediaType string
	bytes     []byte
	failure   *string
}

type ImageSource struct {
	URL    *url.URL
	Images []string
}

type discoveredPage struct {
	url      *url.URL
	key      string
	html     string
	analysis *ChapterAnalysis
}

func convertCrawl(input CrawlInput) (*ConversionResult, error) {
	startURL, err := url.Parse(input.StartURL)
	if err != nil {
		return nil, NewInvalidSourceURLError("Start URL must be absolute HTTP or HTTPS.")
	}
	if startURL.Scheme != "http" && startURL.Scheme != "https" {
		return nil, NewInvalidSourceURLError("Source URL must use HTTP or HTTPS.")
	}

	prefixURL, err := validatePrefixURL(input.Crawl.PrefixURL, startURL)
	if err != nil {
		return nil, err
	}

	metadata := sanitizeMetadata(input.Metadata, input.StartURL)
	pageLookup := buildPageLookup(input.Pages)
	resourceLookup := buildResourceLookup(input.Resources)

	var warnings []ConversionWarning
	warningKeys := make(map[string]bool)

	discoveredPages := discoverPages(startURL, prefixURL, &metadata, &input.Crawl, pageLookup, &warnings, warningKeys)
	if len(discoveredPages) == 0 {
		return nil, NewNoReadableContentError()
	}

	var resources []EpubResource
	var imagePaths map[string]string

	if input.Options.IncludeImages {
		var imageSources []ImageSource
		for _, p := range discoveredPages {
			imageSources = append(imageSources, ImageSource{
				URL:    p.url,
				Images: p.analysis.Images,
			})
		}
		resources, imagePaths = collectImageResources(imageSources, resourceLookup, &warnings, warningKeys)
	} else {
		imagePaths = make(map[string]string)
	}

	chapterPaths := make(map[string]string)
	chapterIDs := make(map[string]map[string]bool)
	for i, p := range discoveredPages {
		path := chapterHref(i)
		chapterPaths[p.key] = path
		chapterIDs[p.key] = p.analysis.IDs
	}

	linkRewrites := &LinkRewriteContext{
		ChapterPaths: chapterPaths,
		ChapterIDs:   chapterIDs,
	}
	imageRewrites := &ImageRewriteContext{
		PackagedPaths: imagePaths,
	}

	var chapters []*Chapter
	for i, p := range discoveredPages {
		ch, err := renderChapter(
			p.html,
			p.url,
			&metadata,
			&input.Options,
			i+1,
			p.analysis.Title,
			linkRewrites,
			imageRewrites,
		)
		if err != nil {
			return nil, err
		}
		ch.NavigationPath = crawlNavigationPath(p.url, prefixURL, p.analysis.Title)
		warnings = append(warnings, ch.Warnings...)
		chapters = append(chapters, ch)
	}

	epubBytes, err := generateEPUB(&metadata, chapters, resources)
	if err != nil {
		return nil, NewEpubGenerationError(err.Error())
	}

	downloadFilename := SafeDownloadFilename(metadata.Title)

	return &ConversionResult{
		EPUBBytes:        epubBytes,
		DownloadFilename: downloadFilename,
		ChapterCount:     len(chapters),
		Metadata:         metadata,
		Warnings:         warnings,
	}, nil
}

func discoverPages(
	startURL, prefixURL *url.URL,
	metadata *SanitizedMetadata,
	crawlOptions *CrawlOptions,
	pageLookup map[string]*pageSource,
	warnings *[]ConversionWarning,
	warningKeys map[string]bool,
) []*discoveredPage {
	pageLimit := crawlOptions.MaxPages
	if pageLimit < 1 {
		pageLimit = 1
	}
	byteLimit := crawlOptions.MaxTotalBytes
	if byteLimit < 1 {
		byteLimit = 1
	}
	timeLimit := time.Duration(crawlOptions.MaxDurationMillis) * time.Millisecond
	started := time.Now()

	var discovered []*discoveredPage
	type queueEntry struct {
		url   *url.URL
		depth int
	}
	queue := []queueEntry{{url: withoutFragment(startURL), depth: 0}}
	seen := make(map[string]bool)
	startKey := normalizePageURL(startURL)
	seen[startKey] = true
	scheduledPages := 1
	totalBytes := 0

	for len(queue) > 0 {
		entry := queue[0]
		queue = queue[1:]
		pageURL := entry.url
		depth := entry.depth

		if crawlOptions.MaxDurationMillis > 0 && time.Since(started) > timeLimit {
			pushWarningOnce(warnings, warningKeys, "crawl_time_limit", "Crawl stopped because the configured time limit was reached.", nil)
			break
		}

		affected := urlWithoutFragment(pageURL)
		key := normalizePageURL(pageURL)
		source, ok := pageLookup[key]
		if !ok {
			continue
		}

		if source.failure != nil {
			switch *source.failure {
			case CrawlTimeLimitFailure:
				pushWarningOnce(warnings, warningKeys, "crawl_time_limit", "Crawl stopped because the configured time limit was reached.", nil)
			case CrawlByteLimitFailure:
				pushWarningOnce(warnings, warningKeys, "crawl_byte_limit", "Pages were skipped because the configured crawl byte limit was reached.", nil)
			default:
				pushWarningOnce(warnings, warningKeys, "page_fetch_failed", "Page was skipped: "+safeWarningDetail(*source.failure), strPtr(affected))
			}
			continue
		}

		if source.html == nil || *source.html == "" {
			pushWarningOnce(warnings, warningKeys, "page_fetch_failed", "Page was skipped because no HTML body was available.", strPtr(affected))
			continue
		}

		htmlStr := *source.html
		if totalBytes+len(htmlStr) > byteLimit {
			pushWarningOnce(warnings, warningKeys, "crawl_byte_limit", "Page was skipped because the configured crawl byte limit was reached.", strPtr(affected))
			continue
		}
		totalBytes += len(htmlStr)

		analysis, err := analyzeChapter(htmlStr, pageURL, metadata)
		if err != nil {
			if convErr, ok := err.(*ConversionError); ok && convErr.Code == "no_readable_content" {
				pushWarningOnce(warnings, warningKeys, "page_no_readable_content", "Page was skipped because it did not contain readable content.", strPtr(affected))
			} else {
				pushWarningOnce(warnings, warningKeys, "page_conversion_failed", err.Error(), strPtr(affected))
			}
			continue
		}

		// MaxDurationMillis == 0 means unlimited duration; the time check above
		// only applies when MaxDurationMillis > 0.
		for _, rawLink := range analysis.Links {
			candidate := resolvePageLink(rawLink, pageURL)
			if candidate == nil {
				continue
			}
			if !isInScope(candidate, startURL, prefixURL) {
				continue
			}
			candidateKey := normalizePageURL(candidate)
			if seen[candidateKey] {
				continue
			}
			if depth+1 > crawlOptions.MaxDepth {
				pushWarningOnce(warnings, warningKeys, "crawl_depth_limit", "Pages were skipped because the configured crawl depth limit was reached.", nil)
				continue
			}
			if scheduledPages >= pageLimit {
				pushWarningOnce(warnings, warningKeys, "crawl_page_limit", "Pages were skipped because the configured crawl page limit was reached.", nil)
				continue
			}
			seen[candidateKey] = true
			scheduledPages++
			queue = append(queue, queueEntry{url: withoutFragment(candidate), depth: depth + 1})
		}

		discovered = append(discovered, &discoveredPage{
			key:      normalizePageURL(pageURL),
			url:      cloneURL(pageURL),
			html:     htmlStr,
			analysis: analysis,
		})
	}

	return discovered
}

func collectImageResources(
	pages []ImageSource,
	resourceLookup map[string]*resourceSource,
	warnings *[]ConversionWarning,
	warningKeys map[string]bool,
) ([]EpubResource, map[string]string) {
	var resources []EpubResource
	packagedPaths := make(map[string]string)
	usedPaths := make(map[string]bool)

	for _, page := range pages {
		for _, rawSrc := range page.Images {
			imageURL, err := resolveImageSrc(rawSrc, page.URL)
			if err != nil {
				pushWarningOnce(warnings, warningKeys, "image_unsupported_scheme", "Image was skipped because its URL scheme is not supported.", strPtr(err.Error()))
				continue
			}

			key := normalizeResourceURL(imageURL)
			if _, exists := packagedPaths[key]; exists {
				continue
			}

			affected := urlWithoutFragment(imageURL)
			source, ok := resourceLookup[key]
			if !ok {
				pushWarningOnce(warnings, warningKeys, "image_fetch_failed", "Image was skipped because it could not be fetched.", strPtr(affected))
				continue
			}

			if source.failure != nil {
				if *source.failure == CrawlTimeLimitFailure {
					continue
				}
				if *source.failure == CrawlByteLimitFailure {
					pushWarningOnce(warnings, warningKeys, "crawl_byte_limit", "Images were skipped because the configured crawl byte limit was reached.", nil)
					continue
				}
				pushWarningOnce(warnings, warningKeys, "image_fetch_failed", "Image was skipped: "+safeWarningDetail(*source.failure), strPtr(affected))
				continue
			}

			if len(source.bytes) == 0 {
				pushWarningOnce(warnings, warningKeys, "image_fetch_failed", "Image was skipped because it had no bytes.", strPtr(affected))
				continue
			}

			mediaType, extension := supportedImageMediaType(source.mediaType, imageURL)
			if mediaType == "" {
				pushWarningOnce(warnings, warningKeys, "image_unsupported_type", "Image was skipped because its media type is not supported.", strPtr(affected))
				continue
			}

			imageBytes := source.bytes
			if isSVGMedia(mediaType, imageBytes) {
				imageBytes = sanitizeSVG(imageBytes)
				if len(imageBytes) == 0 {
					pushWarningOnce(warnings, warningKeys, "image_fetch_failed", "Image was skipped because SVG sanitization removed all content.", strPtr(affected))
					continue
				}
			}

			packagePath := conflictFreeResourcePath(imageURL, extension, usedPaths)
			packagedPaths[key] = "../" + packagePath
			resources = append(resources, EpubResource{
				Path:      packagePath,
				MediaType: mediaType,
				Bytes:     imageBytes,
			})
		}
	}

	return resources, packagedPaths
}

func isSVGMedia(mediaType string, data []byte) bool {
	if strings.Contains(strings.ToLower(mediaType), "svg") {
		return true
	}
	trimmed := bytes.TrimSpace(data)
	if len(trimmed) == 0 {
		return false
	}
	lower := bytes.ToLower(trimmed)
	return bytes.HasPrefix(lower, []byte("<svg")) ||
		(bytes.HasPrefix(lower, []byte("<?xml")) && bytes.Contains(lower, []byte("<svg")))
}

var (
	svgScriptTagRe = regexp.MustCompile(`(?is)<script\b[^>]*>.*?</script\s*>|<script\b[^/]*/>`)
	svgEventAttrRe = regexp.MustCompile(`(?i)\s+on[a-z]+\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+)`)
	svgJSURLRe     = regexp.MustCompile(`(?i)(href|xlink:href|src)\s*=\s*(?:"\s*javascript:[^"]*"|'\s*javascript:[^']*'|\s*javascript:[^\s>]+)`)
)

func sanitizeSVG(data []byte) []byte {
	cleaned := svgScriptTagRe.ReplaceAll(data, nil)
	cleaned = svgEventAttrRe.ReplaceAll(cleaned, nil)
	cleaned = svgJSURLRe.ReplaceAll(cleaned, nil)
	return bytes.TrimSpace(cleaned)
}

func crawlNavigationPath(pageURL, prefixURL *url.URL, title string) []string {
	cleanTitle := sanitizeMetadataValue(title, "Untitled Chapter")
	if pageURL == nil || prefixURL == nil {
		return []string{cleanTitle}
	}

	pagePath := strings.Trim(pageURL.EscapedPath(), "/")
	prefixPath := strings.Trim(prefixURL.EscapedPath(), "/")
	if prefixPath != "" {
		if pagePath == prefixPath {
			return []string{cleanTitle}
		}
		prefixPath += "/"
		if !strings.HasPrefix(pagePath, prefixPath) {
			return []string{cleanTitle}
		}
		pagePath = strings.TrimPrefix(pagePath, prefixPath)
	}

	pagePath = strings.Trim(pagePath, "/")
	if pagePath == "" {
		return []string{cleanTitle}
	}

	segments := strings.Split(pagePath, "/")
	groupSegments := segments[:len(segments)-1]
	if len(groupSegments) == 0 && strings.HasSuffix(pageURL.Path, "/") {
		groupSegments = segments
	}

	navPath := make([]string, 0, len(groupSegments)+1)
	for _, segment := range groupSegments {
		if label := humanizeURLSegment(segment); label != "" {
			navPath = append(navPath, label)
		}
	}
	navPath = append(navPath, cleanTitle)
	return navPath
}

func humanizeURLSegment(segment string) string {
	decoded, err := url.PathUnescape(segment)
	if err != nil {
		decoded = segment
	}
	decoded = strings.TrimSpace(decoded)
	if decoded == "" {
		return ""
	}
	decoded = strings.TrimSuffix(decoded, ".html")
	decoded = strings.TrimSuffix(decoded, ".htm")
	decoded = strings.NewReplacer("-", " ", "_", " ").Replace(decoded)
	words := strings.Fields(decoded)
	for i, word := range words {
		words[i] = humanizeURLWord(word)
	}
	return strings.Join(words, " ")
}

func humanizeURLWord(word string) string {
	if word == "" {
		return ""
	}
	lower := strings.ToLower(word)
	runes := []rune(lower)
	if len(runes) <= 3 {
		return strings.ToUpper(lower)
	}
	return strings.ToUpper(string(runes[:1])) + string(runes[1:])
}

func buildPageLookup(pages []CrawlPage) map[string]*pageSource {
	lookup := make(map[string]*pageSource)
	for _, p := range pages {
		u, err := url.Parse(p.URL)
		if err != nil {
			continue
		}
		key := normalizePageURL(u)
		if _, exists := lookup[key]; !exists {
			lookup[key] = &pageSource{html: p.HTML, failure: p.Failure}
		}
	}
	return lookup
}

func buildResourceLookup(resources []CrawlResource) map[string]*resourceSource {
	lookup := make(map[string]*resourceSource)
	for _, r := range resources {
		u, err := url.Parse(r.URL)
		if err != nil {
			continue
		}
		if u.Scheme != "http" && u.Scheme != "https" {
			continue
		}
		key := normalizeResourceURL(u)
		if _, exists := lookup[key]; !exists {
			lookup[key] = &resourceSource{mediaType: r.MediaType, bytes: r.Bytes, failure: r.Failure}
		}
	}
	return lookup
}

func validatePrefixURL(rawPrefix string, startURL *url.URL) (*url.URL, error) {
	prefix := strings.TrimSpace(rawPrefix)
	if prefix == "" {
		prefix = defaultPrefixFor(startURL)
	}

	parsed, err := url.Parse(prefix)
	if err != nil || (parsed.Scheme != "http" && parsed.Scheme != "https") {
		return nil, NewInvalidSourceURLError("Crawl prefix must be a valid HTTP or HTTPS URL.")
	}

	if !sameOrigin(parsed, startURL) {
		return nil, NewInvalidSourceURLError("Crawl prefix must use the same origin as the source URL.")
	}

	if !isInScope(startURL, startURL, parsed) {
		return nil, NewInvalidSourceURLError("Source URL must be within the configured crawl prefix.")
	}

	normalized := normalizePageURL(parsed)
	parsed, err = url.Parse(normalized)
	if err != nil {
		return nil, NewInvalidSourceURLError("Crawl prefix must be a valid HTTP or HTTPS URL.")
	}
	return parsed, nil
}

func resolvePageLink(rawHref string, sourceURL *url.URL) *url.URL {
	href := strings.TrimSpace(rawHref)
	if href == "" || strings.HasPrefix(href, "#") || containsControl(href) {
		return nil
	}

	resolved, err := sourceURL.Parse(href)
	if err != nil {
		return nil
	}
	if resolved.Scheme != "http" && resolved.Scheme != "https" {
		return nil
	}
	return resolved
}

func resolveImageSrc(rawSrc string, sourceURL *url.URL) (*url.URL, error) {
	src := strings.TrimSpace(rawSrc)
	if src == "" || containsControl(src) {
		return nil, &ConversionError{Code: "invalid", Message: src}
	}

	resolved, err := sourceURL.Parse(src)
	if err != nil {
		return nil, &ConversionError{Code: "invalid", Message: src}
	}
	if resolved.Scheme != "http" && resolved.Scheme != "https" {
		return nil, &ConversionError{Code: "invalid", Message: urlWithoutFragment(resolved)}
	}
	return resolved, nil
}

func isInScope(candidate, startURL, prefixURL *url.URL) bool {
	if !sameOrigin(candidate, startURL) || !sameOrigin(candidate, prefixURL) {
		return false
	}
	normalizedCandidate, err := url.Parse(normalizePageURL(candidate))
	if err != nil {
		return false
	}
	normalizedPrefix, err := url.Parse(normalizePageURL(prefixURL))
	if err != nil {
		return false
	}
	return strings.HasPrefix(normalizedCandidate.Path, normalizedPrefix.Path)
}

func withoutFragment(u *url.URL) *url.URL {
	nu := *u
	nu.Fragment = ""
	nu.User = nil
	return &nu
}

func supportedImageMediaType(rawMediaType string, u *url.URL) (string, string) {
	parts := strings.SplitN(rawMediaType, ";", 2)
	mediaType := strings.TrimSpace(strings.ToLower(parts[0]))

	switch mediaType {
	case "image/png":
		return "image/png", "png"
	case "image/jpeg", "image/jpg":
		return "image/jpeg", "jpg"
	case "image/gif":
		return "image/gif", "gif"
	case "image/webp":
		return "image/webp", "webp"
	case "image/svg+xml":
		return "image/svg+xml", "svg"
	case "":
		ext := pathExtension(u)
		switch ext {
		case "png":
			return "image/png", "png"
		case "jpg", "jpeg":
			return "image/jpeg", "jpg"
		case "gif":
			return "image/gif", "gif"
		case "webp":
			return "image/webp", "webp"
		case "svg":
			return "image/svg+xml", "svg"
		}
	}
	return "", ""
}

func conflictFreeResourcePath(u *url.URL, extension string, usedPaths map[string]bool) string {
	stem := safeResourceStem(u)
	hash := stableHash64(normalizeResourceURL(u))
	path := stem + "-" + fmt.Sprintf("%016x", hash) + "." + extension
	suffix := 2
	for usedPaths[path] {
		path = stem + "-" + fmt.Sprintf("%016x", hash) + "-" + fmt.Sprintf("%d", suffix) + "." + extension
		suffix++
	}
	usedPaths[path] = true
	return "images/" + path
}

func safeResourceStem(u *url.URL) string {
	path := u.Path
	lastSlash := strings.LastIndexByte(path, '/')
	lastSegment := path[lastSlash+1:]

	var stem string
	if idx := strings.LastIndexByte(lastSegment, '.'); idx >= 0 {
		stem = lastSegment[:idx]
	} else {
		stem = lastSegment
	}

	var safe strings.Builder
	previousSep := false
	count := 0
	for _, ch := range stem {
		if count >= 48 {
			break
		}
		if (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || (ch >= '0' && ch <= '9') {
			safe.WriteByte(byte(ch | 32))
			previousSep = false
			count++
		} else if !previousSep {
			safe.WriteByte('-')
			previousSep = true
			count++
		}
	}

	result := strings.Trim(safe.String(), "-")
	if result == "" {
		return "resource"
	}
	return result
}

func pathExtension(u *url.URL) string {
	path := u.Path
	lastSlash := strings.LastIndexByte(path, '/')
	lastSegment := path[lastSlash+1:]
	if idx := strings.LastIndexByte(lastSegment, '.'); idx >= 0 {
		return strings.ToLower(lastSegment[idx+1:])
	}
	return ""
}

func stableHash64(value string) uint64 {
	var hash uint64 = 0xcbf29ce484222325
	for _, b := range []byte(value) {
		hash ^= uint64(b)
		hash *= 0x100000001b3
	}
	return hash
}

func safeWarningDetail(raw string) string {
	var safe strings.Builder
	for _, ch := range raw {
		if ch < 32 {
			safe.WriteByte(' ')
		} else {
			safe.WriteRune(ch)
		}
	}
	result := collapseWhitespace(safe.String())
	runes := []rune(result)
	if len(runes) > 160 {
		return string(runes[:160])
	}
	return result
}

func pushWarningOnce(warnings *[]ConversionWarning, warningKeys map[string]bool, code, message string, affected *string) {
	key := code
	if affected != nil {
		key += ":" + *affected
	}
	if warningKeys[key] {
		return
	}
	warningKeys[key] = true
	*warnings = append(*warnings, ConversionWarning{
		Code:     code,
		Message:  message,
		Affected: affected,
	})
}

func strPtr(s string) *string {
	return &s
}
