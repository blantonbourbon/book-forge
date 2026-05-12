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

func escapeXMLText(input string) string {
	var output strings.Builder
	for _, ch := range input {
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
