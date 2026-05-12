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
	for i, ch := range chapters {
		entries.WriteString(fmt.Sprintf(
			`      <li><a href="%s">%s</a></li>`+"\n",
			escapeXMLAttr(chapterHref(i)), escapeXMLText(ch.Title),
		))
	}

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

func chapterHref(index int) string {
	return fmt.Sprintf("chapters/chapter-%d.xhtml", index+1)
}
