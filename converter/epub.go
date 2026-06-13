package converter

import (
	"archive/zip"
	"bytes"
	"fmt"
	"strings"
)

const navPath = "EPUB/nav.xhtml"
const packagePath = "EPUB/package.opf"

type EpubResource struct {
	Path      string
	MediaType string
	Bytes     []byte
}

type navEntry struct {
	Title    string
	Href     string
	Children []*navEntry
}

func generateSingleEPUB(metadata *SanitizedMetadata, chapter *Chapter, resources []EpubResource) ([]byte, error) {
	return generateEPUB(metadata, []*Chapter{chapter}, resources)
}

func generateEPUB(metadata *SanitizedMetadata, chapters []*Chapter, resources []EpubResource) ([]byte, error) {
	var buf bytes.Buffer
	w := zip.NewWriter(&buf)

	fw, err := w.CreateHeader(&zip.FileHeader{
		Name:   "mimetype",
		Method: zip.Store,
	})
	if err != nil {
		return nil, err
	}
	fw.Write([]byte("application/epub+zip"))

	fw, err = w.Create("META-INF/container.xml")
	if err != nil {
		return nil, err
	}
	fw.Write([]byte(containerXML()))

	fw, err = w.Create(packagePath)
	if err != nil {
		return nil, err
	}
	fw.Write([]byte(packageOPF(metadata, chapters, resources)))

	fw, err = w.Create(navPath)
	if err != nil {
		return nil, err
	}
	fw.Write([]byte(navXHTML(metadata.Language, chapters)))

	for i, ch := range chapters {
		name := chapterHref(i)
		fw, err = w.Create("EPUB/" + name)
		if err != nil {
			return nil, err
		}
		fw.Write([]byte(ch.XHTML))
	}

	for _, res := range resources {
		fw, err = w.Create("EPUB/" + res.Path)
		if err != nil {
			return nil, err
		}
		fw.Write(res.Bytes)
	}

	if err := w.Close(); err != nil {
		return nil, err
	}
	return buf.Bytes(), nil
}

func containerXML() string {
	return `<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="EPUB/package.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
`
}

func packageOPF(metadata *SanitizedMetadata, chapters []*Chapter, resources []EpubResource) string {
	description := ""
	if metadata.Description != "" {
		description = fmt.Sprintf("    <dc:description>%s</dc:description>\n", escapeXMLText(metadata.Description))
	}

	var manifestItems strings.Builder
	for i := range chapters {
		manifestItems.WriteString(fmt.Sprintf(
			`    <item id="chapter-%d" href="%s" media-type="application/xhtml+xml"/>`+"\n",
			i+1, escapeXMLAttr(chapterHref(i)),
		))
	}
	for i, res := range resources {
		manifestItems.WriteString(fmt.Sprintf(
			`    <item id="resource-%d" href="%s" media-type="%s"/>`+"\n",
			i+1, escapeXMLAttr(res.Path), escapeXMLAttr(res.MediaType),
		))
	}

	var spineItems strings.Builder
	for i := range chapters {
		spineItems.WriteString(fmt.Sprintf(`    <itemref idref="chapter-%d"/>`+"\n", i+1))
	}

	return fmt.Sprintf(`<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="book-id" xml:lang="%s">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="book-id">%s</dc:identifier>
    <dc:title>%s</dc:title>
    <dc:creator id="creator">%s</dc:creator>
    <dc:language>%s</dc:language>
%s    <meta property="dcterms:modified">%s</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
%s  </manifest>
  <spine>
%s  </spine>
</package>
`,
		escapeXMLAttr(metadata.Language),
		escapeXMLText(metadata.Identifier),
		escapeXMLText(metadata.Title),
		escapeXMLText(metadata.Author),
		escapeXMLAttr(metadata.Language),
		description,
		escapeXMLText(metadata.Modified),
		manifestItems.String(),
		spineItems.String(),
	)
}

func navXHTML(language string, chapters []*Chapter) string {
	var entries strings.Builder
	renderNavEntries(&entries, buildNavEntries(chapters), 3)

	return fmt.Sprintf(`<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" lang="%s" xml:lang="%s">
<head>
  <meta charset="utf-8" />
  <title>Table of Contents</title>
</head>
<body>
  <nav epub:type="toc" id="toc">
    <h1>Table of Contents</h1>
    <ol>
%s    </ol>
  </nav>
</body>
</html>
`, escapeXMLAttr(language), escapeXMLAttr(language), entries.String())
}

func buildNavEntries(chapters []*Chapter) []*navEntry {
	var roots []*navEntry
	for i, ch := range chapters {
		path := normalizedNavigationPath(ch)
		href := chapterHref(i)
		parent := &roots
		for _, segment := range path[:len(path)-1] {
			group := findNavEntry(*parent, segment)
			if group == nil {
				group = &navEntry{Title: segment}
				*parent = append(*parent, group)
			}
			parent = &group.Children
		}
		*parent = append(*parent, &navEntry{
			Title: path[len(path)-1],
			Href:  href,
		})
	}
	return roots
}

func normalizedNavigationPath(ch *Chapter) []string {
	var path []string
	for _, segment := range ch.NavigationPath {
		clean := sanitizeMetadataValue(segment, "")
		if clean != "" {
			path = append(path, clean)
		}
	}
	if len(path) == 0 {
		path = append(path, sanitizeMetadataValue(ch.Title, "Untitled Chapter"))
	}
	return path
}

func findNavEntry(entries []*navEntry, title string) *navEntry {
	for _, entry := range entries {
		if entry.Href == "" && entry.Title == title {
			return entry
		}
	}
	return nil
}

func renderNavEntries(output *strings.Builder, entries []*navEntry, indentLevel int) {
	for _, entry := range entries {
		indent := strings.Repeat("  ", indentLevel)
		output.WriteString(indent)
		output.WriteString("<li>")
		if entry.Href != "" {
			output.WriteString(`<a href="`)
			output.WriteString(escapeXMLAttr(entry.Href))
			output.WriteString(`">`)
			output.WriteString(escapeXMLText(entry.Title))
			output.WriteString("</a>")
		} else {
			output.WriteString("<span>")
			output.WriteString(escapeXMLText(entry.Title))
			output.WriteString("</span>")
		}
		if len(entry.Children) == 0 {
			output.WriteString("</li>\n")
			continue
		}
		output.WriteString("\n")
		output.WriteString(indent)
		output.WriteString("  <ol>\n")
		renderNavEntries(output, entry.Children, indentLevel+2)
		output.WriteString(indent)
		output.WriteString("  </ol>\n")
		output.WriteString(indent)
		output.WriteString("</li>\n")
	}
}

func chapterHref(index int) string {
	return fmt.Sprintf("chapters/chapter-%d.xhtml", index+1)
}
