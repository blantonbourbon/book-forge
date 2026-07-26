package converter

import "strings"

func collapseWhitespace(input string) string {
	fields := strings.Fields(input)
	return strings.Join(fields, " ")
}

func stripHTMLTags(raw string) string {
	var output strings.Builder
	output.Grow(len(raw))
	insideTag := false

	for _, ch := range raw {
		switch {
		case ch == '<':
			insideTag = true
			output.WriteByte(' ')
		case ch == '>' && insideTag:
			insideTag = false
			output.WriteByte(' ')
		case insideTag:
		default:
			output.WriteRune(ch)
		}
	}

	return output.String()
}

// isXMLChar reports whether ch is a legal XML 1.0 character.
// Allowed: #x9 | #xA | #xD | [#x20-#xD7FF] | [#xE000-#xFFFD] | [#x10000-#x10FFFF]
func isXMLChar(ch rune) bool {
	switch {
	case ch == 0x9 || ch == 0xA || ch == 0xD:
		return true
	case ch >= 0x20 && ch <= 0xD7FF:
		return true
	case ch >= 0xE000 && ch <= 0xFFFD:
		return true
	case ch >= 0x10000 && ch <= 0x10FFFF:
		return true
	default:
		return false
	}
}

func escapeXMLText(input string) string {
	var output strings.Builder
	for _, ch := range input {
		if !isXMLChar(ch) {
			continue
		}
		switch ch {
		case '&':
			output.WriteString("&amp;")
		case '<':
			output.WriteString("&lt;")
		case '>':
			output.WriteString("&gt;")
		default:
			output.WriteRune(ch)
		}
	}
	return output.String()
}

func escapeXMLAttr(input string) string {
	var output strings.Builder
	for _, ch := range input {
		if !isXMLChar(ch) {
			continue
		}
		switch ch {
		case '&':
			output.WriteString("&amp;")
		case '<':
			output.WriteString("&lt;")
		case '>':
			output.WriteString("&gt;")
		case '"':
			output.WriteString("&quot;")
		case '\'':
			output.WriteString("&apos;")
		default:
			output.WriteRune(ch)
		}
	}
	return output.String()
}
