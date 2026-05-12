package server

import (
	"net/http"
	"net/url"
	"strings"
	"time"

	"github.com/PuerkitoBio/goquery"
	"github.com/gin-gonic/gin"
)

const metadataPreviewTimeout = 8 * time.Second

type MetadataPreviewResponse struct {
	Title       string `json:"title"`
	Author      string `json:"author"`
	Description string `json:"description"`
	FinalURL    string `json:"finalUrl"`
}

func handlePreviewMetadata(state *AppState) gin.HandlerFunc {
	return func(c *gin.Context) {
		rawURL := strings.TrimSpace(c.Query("url"))
		if rawURL == "" {
			RespondError(c, ValidationError("A source URL is required.", []string{"sourceUrl"}))
			return
		}
		if parsed, err := url.Parse(rawURL); err != nil || (parsed.Scheme != "http" && parsed.Scheme != "https") {
			RespondError(c, ValidationError("Only HTTP and HTTPS source URLs are supported.", []string{"sourceUrl"}))
			return
		}
		if err := ValidateNetworkURL(rawURL); err != nil {
			RespondError(c, ValidationError(err.Error(), []string{"sourceUrl"}))
			return
		}

		var fetched *FetchedResponse
		var err error
		if strings.EqualFold(c.Query("useBrowser"), "true") && state.BrowserFetcher != nil {
			fetched, err = fetchHTMLBrowser(state.BrowserFetcher, rawURL, metadataPreviewTimeout, defaultMaxTotalBytes)
		} else {
			fetched, err = fetchHTML(state.Fetcher, rawURL, metadataPreviewTimeout, defaultMaxTotalBytes)
		}
		if err != nil {
			if apiErr, ok := err.(*FetchError); ok {
				RespondError(c, NewAPIError(http.StatusBadGateway, apiErr.Code, apiErr.Message))
				return
			}
			RespondError(c, NewAPIError(http.StatusBadGateway, "preview_fetch_failed", "Metadata could not be fetched from the source URL."))
			return
		}

		html, _ := fetched.Text()
		doc, err := goquery.NewDocumentFromReader(strings.NewReader(html))
		if err != nil {
			RespondError(c, NewAPIError(http.StatusBadGateway, "preview_parse_failed", "Metadata could not be read from the source HTML."))
			return
		}

		title := firstNonEmpty(
			metaContent(doc, `meta[property="og:title"]`),
			metaContent(doc, `meta[name="twitter:title"]`),
			doc.Find("title").First().Text(),
			doc.Find("h1").First().Text(),
		)
		author := firstNonEmpty(
			metaContent(doc, `meta[name="author"]`),
			metaContent(doc, `meta[property="article:author"]`),
			strings.TrimPrefix(metaContent(doc, `meta[name="twitter:creator"]`), "@"),
		)
		description := firstNonEmpty(
			metaContent(doc, `meta[name="description"]`),
			metaContent(doc, `meta[property="og:description"]`),
			metaContent(doc, `meta[name="twitter:description"]`),
		)

		c.JSON(http.StatusOK, MetadataPreviewResponse{
			Title:       cleanPreviewText(title, maxMetadataChars),
			Author:      cleanPreviewText(author, maxMetadataChars),
			Description: cleanPreviewText(description, maxMetadataChars),
			FinalURL:    fetched.FinalURL,
		})
	}
}

func metaContent(doc *goquery.Document, selector string) string {
	content, exists := doc.Find(selector).First().Attr("content")
	if !exists {
		return ""
	}
	return content
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		cleaned := cleanPreviewText(value, maxMetadataChars)
		if cleaned != "" {
			return cleaned
		}
	}
	return ""
}

func cleanPreviewText(value string, maxRunes int) string {
	value = strings.TrimSpace(strings.Join(strings.Fields(value), " "))
	if value == "" {
		return ""
	}
	runes := []rune(value)
	if len(runes) > maxRunes {
		return string(runes[:maxRunes])
	}
	return value
}
