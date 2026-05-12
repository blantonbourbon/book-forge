package converter

import (
	"net/url"
)

func ConvertSinglePage(input SinglePageInput) (*ConversionResult, error) {
	sourceURL, err := url.Parse(input.SourceURL)
	if err != nil || (sourceURL.Scheme != "http" && sourceURL.Scheme != "https") {
		return nil, NewInvalidSourceURLError("Source URL must be an absolute HTTP or HTTPS URL.")
	}

	metadata := sanitizeMetadata(input.Metadata, input.SourceURL)
	analysis, err := analyzeChapter(input.HTML, sourceURL, &metadata)
	if err != nil {
		return nil, err
	}

	var resources []EpubResource
	var imagePaths map[string]string
	warningKeys := make(map[string]bool)
	var warnings []ConversionWarning

	if input.Options.IncludeImages {
		resourceLookup := buildResourceLookup(input.Resources)
		resources, imagePaths = collectImageResources(
			[]ImageSource{{URL: sourceURL, Images: analysis.Images}},
			resourceLookup,
			&warnings,
			warningKeys,
		)
	} else {
		imagePaths = make(map[string]string)
	}

	var chapterRewrites *ImageRewriteContext
	if input.Options.IncludeImages {
		chapterRewrites = &ImageRewriteContext{PackagedPaths: imagePaths}
	}

	chapter, err := renderChapter(
		input.HTML,
		sourceURL,
		&metadata,
		&input.Options,
		1,
		analysis.Title,
		nil,
		chapterRewrites,
	)
	if err != nil {
		return nil, err
	}
	warnings = append(warnings, chapter.Warnings...)

	epubBytes, err := generateSingleEPUB(&metadata, chapter, resources)
	if err != nil {
		return nil, NewEpubGenerationError(err.Error())
	}

	downloadFilename := SafeDownloadFilename(metadata.Title)

	return &ConversionResult{
		EPUBBytes:        epubBytes,
		DownloadFilename: downloadFilename,
		ChapterCount:     1,
		Metadata:         metadata,
		Warnings:         warnings,
	}, nil
}

func ConvertCrawl(input CrawlInput) (*ConversionResult, error) {
	return convertCrawl(input)
}
