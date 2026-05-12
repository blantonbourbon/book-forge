package converter

import (
	"fmt"
	"net/url"
	"strings"

	"golang.org/x/net/html"
)

type Chapter struct {
	Title    string
	XHTML    string
	Warnings []ConversionWarning
}

type ChapterAnalysis struct {
	Title  string
	IDs    map[string]bool
	Links  []string
	Images []string
}

type LinkRewriteContext struct {
	ChapterPaths map[string]string
	ChapterIDs   map[string]map[string]bool
}

type ImageRewriteContext struct {
	PackagedPaths map[string]string
}

func analyzeChapter(rawHTML string, sourceURL *url.URL, metadata *SanitizedMetadata) (*ChapterAnalysis, error) {
	doc, err := html.Parse(strings.NewReader(rawHTML))
	if err != nil {
		return nil, NewNoReadableContentError()
	}

	root := selectReadingRoot(doc)
	if root == nil {
		return nil, NewNoReadableContentError()
	}

	ids := collectIDs(root)
	visibleText := collapseWhitespace(visibleTextForChildren(root))
	if visibleText == "" {
		return nil, NewNoReadableContentError()
	}

	title := firstHeading(root)
	if title == "" {
		title = documentTitle(doc)
	}
	if title == "" {
		title = metadata.Title
	}
	title = sanitizeMetadataValue(title, metadata.Title)

	return &ChapterAnalysis{
		Title:  title,
		IDs:    ids,
		Links:  collectAttrValues(root, "a", "href"),
		Images: collectAttrValues(root, "img", "src"),
	}, nil
}

func renderChapter(
	rawHTML string,
	sourceURL *url.URL,
	metadata *SanitizedMetadata,
	options *ConversionOptions,
	chapterNumber int,
	title string,
	linkRewrites *LinkRewriteContext,
	imageRewrites *ImageRewriteContext,
) (*Chapter, error) {
	doc, err := html.Parse(strings.NewReader(rawHTML))
	if err != nil {
		return nil, NewNoReadableContentError()
	}

	root := selectReadingRoot(doc)
	if root == nil {
		return nil, NewNoReadableContentError()
	}

	ids := collectIDs(root)
	ctx := &renderContext{
		sourceURL:     sourceURL,
		ids:           ids,
		includeImages: options.IncludeImages,
		linkRewrites:  linkRewrites,
		imageRewrites: imageRewrites,
	}

	var body strings.Builder
	for c := root.FirstChild; c != nil; c = c.NextSibling {
		renderNode(c, ctx, &body)
	}

	if collapseWhitespace(stripHTMLTags(body.String())) == "" {
		return nil, NewNoReadableContentError()
	}

	xhtml := chapterDocument(metadata.Language, title, chapterNumber, body.String())

	return &Chapter{
		Title:    title,
		XHTML:    xhtml,
		Warnings: nil,
	}, nil
}

func selectReadingRoot(doc *html.Node) *html.Node {
	for _, tag := range []string{"article", "main", "body"} {
		if el := findElement(doc, tag); el != nil {
			return el
		}
	}
	return nil
}

func findElement(n *html.Node, tag string) *html.Node {
	if n.Type == html.ElementNode && n.Data == tag {
		return n
	}
	for c := n.FirstChild; c != nil; c = c.NextSibling {
		if el := findElement(c, tag); el != nil {
			return el
		}
	}
	return nil
}

func firstHeading(root *html.Node) string {
	for _, tag := range []string{"h1", "h2", "h3", "h4", "h5", "h6"} {
		if el := findElement(root, tag); el != nil {
			text := collectText(el)
			title := sanitizeMetadataValue(text, "")
			if title != "" {
				return title
			}
		}
	}
	return ""
}

func documentTitle(doc *html.Node) string {
	if el := findElement(doc, "title"); el != nil {
		text := collectText(el)
		return sanitizeMetadataValue(text, "")
	}
	return ""
}

func collectText(n *html.Node) string {
	var buf strings.Builder
	var walk func(*html.Node)
	walk = func(node *html.Node) {
		if node.Type == html.TextNode {
			buf.WriteString(node.Data)
		}
		for c := node.FirstChild; c != nil; c = c.NextSibling {
			walk(c)
		}
	}
	walk(n)
	return buf.String()
}

type renderContext struct {
	sourceURL     *url.URL
	ids           map[string]bool
	includeImages bool
	linkRewrites  *LinkRewriteContext
	imageRewrites *ImageRewriteContext
}

func collectIDs(root *html.Node) map[string]bool {
	ids := make(map[string]bool)
	var walk func(*html.Node)
	walk = func(n *html.Node) {
		if n.Type == html.ElementNode {
			for _, attr := range n.Attr {
				if attr.Key == "id" {
					if id := sanitizeID(attr.Val); id != "" {
						ids[id] = true
					}
				}
			}
		}
		for c := n.FirstChild; c != nil; c = c.NextSibling {
			walk(c)
		}
	}
	walk(root)
	return ids
}

func collectAttrValues(root *html.Node, element, attr string) []string {
	var values []string
	var walk func(*html.Node)
	walk = func(n *html.Node) {
		if n.Type == html.ElementNode && n.Data == element {
			for _, a := range n.Attr {
				if a.Key == attr {
					val := strings.TrimSpace(a.Val)
					if val != "" && !containsControl(val) {
						values = append(values, val)
					}
				}
			}
		}
		for c := n.FirstChild; c != nil; c = c.NextSibling {
			walk(c)
		}
	}
	walk(root)
	return values
}

func containsControl(s string) bool {
	for _, ch := range s {
		if ch < 32 {
			return true
		}
	}
	return false
}

func renderNode(n *html.Node, ctx *renderContext, output *strings.Builder) {
	switch n.Type {
	case html.TextNode:
		output.WriteString(escapeXMLText(n.Data))
	case html.ElementNode:
		name := n.Data

		if isActiveOrUnsafe(name) {
			return
		}

		if name == "a" {
			renderAnchor(n, ctx, output)
			return
		}

		if name == "img" {
			renderImageAlt(n, ctx, output)
			return
		}

		tag, ok := safeXHTMLTag(name)
		if !ok {
			renderChildren(n, ctx, output)
			return
		}

		if tag == "br" || tag == "hr" {
			output.WriteString("<")
			output.WriteString(tag)
			renderIDAttr(n, output)
			output.WriteString(" />")
			return
		}

		output.WriteString("<")
		output.WriteString(tag)
		renderIDAttr(n, output)
		if tag == "th" || tag == "td" {
			renderScopeAttr(n, output)
		}
		output.WriteString(">")
		renderChildren(n, ctx, output)
		output.WriteString("</")
		output.WriteString(tag)
		output.WriteString(">")
	}
}

func renderChildren(n *html.Node, ctx *renderContext, output *strings.Builder) {
	for c := n.FirstChild; c != nil; c = c.NextSibling {
		renderNode(c, ctx, output)
	}
}

func renderAnchor(n *html.Node, ctx *renderContext, output *strings.Builder) {
	var childHTML strings.Builder
	renderChildren(n, ctx, &childHTML)

	if strings.TrimSpace(childHTML.String()) == "" {
		return
	}

	href := getAttr(n, "href")
	if safeHref := makeSafeHref(href, ctx.sourceURL, ctx.ids, ctx.linkRewrites); safeHref != "" {
		output.WriteString(`<a href="`)
		output.WriteString(escapeXMLAttr(safeHref))
		output.WriteString(`">`)
		output.WriteString(childHTML.String())
		output.WriteString("</a>")
	} else {
		output.WriteString(childHTML.String())
	}
}

func renderImageAlt(n *html.Node, ctx *renderContext, output *strings.Builder) {
	alt := sanitizeMetadataValue(getAttr(n, "alt"), "")

	if ctx.includeImages {
		src := getAttr(n, "src")
		if safeSrc := makeSafeImageSrc(src, ctx.sourceURL, ctx.imageRewrites); safeSrc != "" {
			output.WriteString(`<img src="`)
			output.WriteString(escapeXMLAttr(safeSrc))
			output.WriteString(`" alt="`)
			output.WriteString(escapeXMLAttr(alt))
			output.WriteString(`" />`)
			return
		}
	}

	if alt == "" {
		return
	}

	output.WriteString("<span>")
	output.WriteString(escapeXMLText(alt))
	output.WriteString("</span>")
}

func renderIDAttr(n *html.Node, output *strings.Builder) {
	if id := sanitizeID(getAttr(n, "id")); id != "" {
		output.WriteString(` id="`)
		output.WriteString(escapeXMLAttr(id))
		output.WriteString(`"`)
	}
}

func renderScopeAttr(n *html.Node, output *strings.Builder) {
	scope := getAttr(n, "scope")
	if scope == "row" || scope == "col" || scope == "rowgroup" || scope == "colgroup" {
		output.WriteString(` scope="`)
		output.WriteString(scope)
		output.WriteString(`"`)
	}
}

func isActiveOrUnsafe(name string) bool {
	switch name {
	case "script", "style", "form", "input", "button", "select",
		"option", "textarea", "iframe", "object", "embed", "applet",
		"canvas", "video", "audio", "source", "track", "meta", "link":
		return true
	}
	return false
}

func safeXHTMLTag(name string) (string, bool) {
	switch name {
	case "h1", "h2", "h3", "h4", "h5", "h6", "p",
		"u", "blockquote", "ol", "ul", "li", "pre", "code",
		"table", "caption", "thead", "tbody", "tfoot", "tr",
		"th", "td", "br", "hr", "span":
		return name, true
	case "strong", "b":
		return "strong", true
	case "em", "i":
		return "em", true
	}
	return "", false
}

func makeSafeHref(rawHref string, sourceURL *url.URL, ids map[string]bool, linkRewrites *LinkRewriteContext) string {
	href := strings.TrimSpace(rawHref)
	if href == "" || containsControl(href) {
		return ""
	}

	if strings.HasPrefix(href, "#") {
		id := sanitizeID(href[1:])
		if id != "" && ids[id] {
			return "#" + id
		}
		return ""
	}

	if linkRewrites != nil {
		return safeCrawlHref(href, sourceURL, ids, linkRewrites)
	}

	u, err := url.Parse(href)
	if err == nil {
		switch u.Scheme {
		case "http", "https":
			if sameDocument(sourceURL, u) {
				if frag := sanitizeID(u.Fragment); frag != "" && ids[frag] {
					return "#" + frag
				}
				return ""
			}
			return href
		case "mailto":
			return href
		default:
			return ""
		}
	}

	resolved, err := sourceURL.Parse(href)
	if err != nil {
		return ""
	}
	frag := sanitizeID(resolved.Fragment)
	if sameDocument(sourceURL, resolved) && frag != "" && ids[frag] {
		return "#" + frag
	}
	return ""
}

func safeCrawlHref(href string, sourceURL *url.URL, ids map[string]bool, rewrites *LinkRewriteContext) string {
	u, err := url.Parse(href)
	if err == nil {
		switch u.Scheme {
		case "http", "https":
			return rewriteHTTPHref(u, sourceURL, ids, rewrites)
		case "mailto":
			return href
		default:
			return ""
		}
	}

	resolved, err := sourceURL.Parse(href)
	if err != nil {
		return ""
	}
	switch resolved.Scheme {
	case "http", "https":
		return rewriteHTTPHref(resolved, sourceURL, ids, rewrites)
	default:
		return ""
	}
}

func rewriteHTTPHref(resolved, sourceURL *url.URL, ids map[string]bool, rewrites *LinkRewriteContext) string {
	targetKey := normalizePageURL(resolved)
	currentKey := normalizePageURL(sourceURL)

	targetPath, exists := rewrites.ChapterPaths[targetKey]
	if !exists {
		return resolved.String()
	}

	frag := sanitizeID(resolved.Fragment)
	var fragValid bool
	if targetKey == currentKey {
		fragValid = frag != "" && ids[frag]
	} else {
		targetIDs, ok := rewrites.ChapterIDs[targetKey]
		fragValid = ok && frag != "" && targetIDs[frag]
	}

	if targetKey == currentKey {
		if fragValid {
			return "#" + frag
		}
		return ""
	}

	if fragValid {
		return targetPath + "#" + frag
	}
	return targetPath
}

func makeSafeImageSrc(rawSrc string, sourceURL *url.URL, imageRewrites *ImageRewriteContext) string {
	src := strings.TrimSpace(rawSrc)
	if src == "" || containsControl(src) {
		return ""
	}

	if imageRewrites == nil {
		return ""
	}

	resolved, err := url.Parse(src)
	if err != nil {
		resolved, err = sourceURL.Parse(src)
	}
	if err != nil {
		return ""
	}

	if resolved.Scheme != "http" && resolved.Scheme != "https" {
		return ""
	}

	path, ok := imageRewrites.PackagedPaths[normalizeResourceURL(resolved)]
	if !ok {
		return ""
	}
	return path
}

func sanitizeID(raw string) string {
	var id strings.Builder
	for _, ch := range strings.TrimSpace(raw) {
		if (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || (ch >= '0' && ch <= '9') || ch == '-' || ch == '_' || ch == ':' || ch == '.' {
			id.WriteRune(ch)
		} else if !strings.HasSuffix(id.String(), "-") {
			id.WriteByte('-')
		}
	}

	result := strings.Trim(id.String(), "-")
	if result == "" {
		return ""
	}

	first := result[0]
	if !((first >= 'a' && first <= 'z') || (first >= 'A' && first <= 'Z') || first == '_') {
		result = "id-" + result
	}

	return result
}

func sameDocument(left, right *url.URL) bool {
	return normalizePageURL(left) == normalizePageURL(right)
}

func visibleTextForChildren(root *html.Node) string {
	var text strings.Builder
	for c := root.FirstChild; c != nil; c = c.NextSibling {
		collectVisibleText(c, &text)
	}
	return text.String()
}

func collectVisibleText(n *html.Node, output *strings.Builder) {
	switch n.Type {
	case html.TextNode:
		output.WriteString(n.Data)
		output.WriteByte(' ')
	case html.ElementNode:
		name := n.Data
		if isActiveOrUnsafe(name) {
			return
		}
		if name == "img" {
			alt := getAttr(n, "alt")
			if alt != "" {
				output.WriteString(alt)
				output.WriteByte(' ')
			}
			return
		}
		for c := n.FirstChild; c != nil; c = c.NextSibling {
			collectVisibleText(c, output)
		}
	}
}

func getAttr(n *html.Node, key string) string {
	for _, a := range n.Attr {
		if a.Key == key {
			return a.Val
		}
	}
	return ""
}

func chapterDocument(language, title string, chapterNumber int, body string) string {
	return fmt.Sprintf(`<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" lang="%s" xml:lang="%s">
<head>
  <meta charset="utf-8" />
  <title>%s</title>
</head>
<body>
  <section id="chapter-%d">
    %s
  </section>
</body>
</html>
`, escapeXMLAttr(language), escapeXMLAttr(language), escapeXMLText(title), chapterNumber, body)
}
