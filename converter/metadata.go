package converter

import (
	"fmt"
	"strings"
)

const modifiedDate = "2026-05-10T00:00:00Z"

type SanitizedMetadata struct {
	Title       string `json:"title"`
	Author      string `json:"author"`
	Language    string `json:"language"`
	Description string `json:"description"`
	Identifier  string `json:"identifier"`
	Modified    string `json:"modified"`
}

func sanitizeMetadata(metadata BookMetadata, sourceURL string) SanitizedMetadata {
	title := sanitizeMetadataValue(metadata.Title, "Untitled Book")
	author := sanitizeMetadataValue(metadata.Author, "Unknown Author")
	language := sanitizeLanguage(metadata.Language)
	description := sanitizeMetadataValue(metadata.Description, "")
	identifier := stableIdentifier(sourceURL, title)

	return SanitizedMetadata{
		Title:       title,
		Author:      author,
		Language:    language,
		Description: description,
		Identifier:  identifier,
		Modified:    modifiedDate,
	}
}

func sanitizeMetadataValue(raw, fallback string) string {
	withoutTags := stripHTMLTags(raw)
	var cleaned strings.Builder
	cleaned.Grow(len(withoutTags))

	for _, ch := range withoutTags {
		if isControl(ch) || strings.ContainsRune("/\\<>\"'", ch) || ch == '\u2028' || ch == '\u2029' {
			cleaned.WriteByte(' ')
		} else {
			cleaned.WriteRune(ch)
		}
	}

	result := cleaned.String()
	for strings.Contains(result, "..") {
		result = strings.ReplaceAll(result, "..", " ")
	}

	collapsed := collapseWhitespace(result)
	if collapsed == "" {
		return fallback
	}
	return collapsed
}

func SafeDownloadFilename(title string) string {
	var filename strings.Builder
	filename.Grow(len(title) + 5)
	previousSep := false

	for _, ch := range title {
		if len([]byte(filename.String())) >= 80 {
			break
		}
		if ch >= 'a' && ch <= 'z' || ch >= 'A' && ch <= 'Z' || ch >= '0' && ch <= '9' {
			filename.WriteByte(byte(ch | 32))
			previousSep = false
		} else if !previousSep {
			filename.WriteByte('-')
			previousSep = true
		}
	}

	name := strings.Trim(filename.String(), "-.")
	if name == "" {
		name = "book-forge"
	}
	return name + ".epub"
}

func sanitizeLanguage(raw string) string {
	var candidate strings.Builder
	for _, ch := range raw {
		if candidate.Len() >= 35 {
			break
		}
		if (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || (ch >= '0' && ch <= '9') || ch == '-' {
			candidate.WriteRune(ch)
		}
	}
	result := strings.TrimSpace(candidate.String())
	if result == "" {
		return "en"
	}
	return result
}

func stableIdentifier(sourceURL, title string) string {
	var hash uint64 = 0xcbf29ce484222325
	for _, b := range []byte(sourceURL) {
		hash ^= uint64(b)
		hash *= 0x100000001b3
	}
	for _, b := range []byte(title) {
		hash ^= uint64(b)
		hash *= 0x100000001b3
	}
	return fmt.Sprintf("urn:book-forge:%016x", hash)
}

func isControl(ch rune) bool {
	return ch < 32 && ch != '\n' && ch != '\r' && ch != '\t'
}
