package main

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/blantonbourbon/book-forge/internal/epubcheck"
)

func main() {
	if len(os.Args) < 2 {
		fmt.Fprintf(os.Stderr, "Usage: inspect-epub <file> [--json]\n")
		os.Exit(1)
	}

	filePath := os.Args[1]
	useJSON := len(os.Args) > 2 && os.Args[2] == "--json"

	f, err := os.Open(filePath)
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
	defer f.Close()

	info, err := f.Stat()
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}

	report, err := epubcheck.InspectEPUB(f, info.Size())
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}

	if useJSON {
		json.NewEncoder(os.Stdout).Encode(report)
		return
	}

	fmt.Printf("EPUB inspection: %s\n", filePath)
	fmt.Printf("  OK: %v\n", report.OK)
	fmt.Printf("  mimetype: %v\n", report.HasMimetype)
	fmt.Printf("  container.xml: %v\n", report.HasContainer)
	fmt.Printf("  package.opf: %v\n", report.HasPackageDoc)
	fmt.Printf("  chapters: %d\n", report.ChapterCount)
	fmt.Printf("  resources: %d\n", report.ResourceCount)
	if len(report.ExternalRefs) > 0 {
		fmt.Printf("  external refs: %d\n", len(report.ExternalRefs))
	}
	if len(report.Errors) > 0 {
		for _, e := range report.Errors {
			fmt.Printf("  ERROR: %s\n", e)
		}
	}
}
