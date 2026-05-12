package epubcheck

import (
	"archive/zip"
	"encoding/xml"
	"fmt"
	"io"
	"strings"
)

type InspectionReport struct {
	OK            bool     `json:"ok"`
	HasMimetype   bool     `json:"hasMimetype"`
	HasContainer  bool     `json:"hasContainer"`
	HasPackageDoc bool     `json:"hasPackageDoc"`
	HasNav        bool     `json:"hasNav"`
	ChapterCount  int      `json:"chapterCount"`
	ResourceCount int      `json:"resourceCount"`
	ExternalRefs  []string `json:"externalRefs,omitempty"`
	Errors        []string `json:"errors,omitempty"`
}

func InspectEPUB(r io.ReaderAt, size int64) (*InspectionReport, error) {
	zr, err := zip.NewReader(r, size)
	if err != nil {
		return nil, fmt.Errorf("failed to open EPUB: %w", err)
	}

	report := &InspectionReport{
		OK: true,
	}

	for _, f := range zr.File {
		switch f.Name {
		case "mimetype":
			report.HasMimetype = true
		case "META-INF/container.xml":
			report.HasContainer = true
		}
	}

	if !report.HasMimetype {
		report.OK = false
		report.Errors = append(report.Errors, "missing mimetype entry")
	}
	if !report.HasContainer {
		report.OK = false
		report.Errors = append(report.Errors, "missing META-INF/container.xml")
	}

	for _, f := range zr.File {
		if strings.EqualFold(f.Name, "EPUB/package.opf") || strings.HasPrefix(strings.ToLower(f.Name), "epub/") && strings.HasSuffix(strings.ToLower(f.Name), ".opf") {
			report.HasPackageDoc = true
			parsePackageDoc(f, report)
		}
	}

	if !report.HasPackageDoc {
		report.OK = false
		report.Errors = append(report.Errors, "missing OPF package document")
	}

	return report, nil
}

func parsePackageDoc(f *zip.File, report *InspectionReport) {
	rc, err := f.Open()
	if err != nil {
		return
	}
	defer rc.Close()

	decoder := xml.NewDecoder(rc)
	for {
		token, err := decoder.Token()
		if err != nil {
			break
		}

		switch el := token.(type) {
		case xml.StartElement:
			switch el.Name.Local {
			case "item":
				var mediaType, href string
				for _, attr := range el.Attr {
					switch attr.Name.Local {
					case "media-type":
						mediaType = attr.Value
					case "href":
						href = attr.Value
					}
				}
				if mediaType == "application/xhtml+xml" {
					report.ChapterCount++
					if strings.HasPrefix(href, "http://") || strings.HasPrefix(href, "https://") {
						report.ExternalRefs = append(report.ExternalRefs, href)
					}
				} else if mediaType != "" {
					report.ResourceCount++
					if strings.HasPrefix(href, "http://") || strings.HasPrefix(href, "https://") {
						report.ExternalRefs = append(report.ExternalRefs, href)
					}
				}
			case "itemref":
				report.ChapterCount = max(report.ChapterCount, report.ChapterCount+0)
			}
		}
	}
}
