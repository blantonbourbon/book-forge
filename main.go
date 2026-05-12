package main

import (
	"log"
	"os"

	"github.com/blantonbourbon/book-forge/server"
)

func main() {
	host := os.Getenv("HOST")
	if host == "" {
		host = "127.0.0.1"
	}
	port := os.Getenv("PORT")
	if port == "" {
		port = "3100"
	}
	staticDir := os.Getenv("STATIC_DIR")

	fetcher := server.NewSharedFetcher()
	state := server.NewAppState(fetcher)
	if staticDir != "" {
		state.StaticRoot = staticDir
	}
	if sidecarURL := os.Getenv("CLOAK_SIDECAR_URL"); sidecarURL != "" {
		state.BrowserFetcher = server.NewBrowserFetcher(sidecarURL)
	}

	r := server.SetupRouter(state)

	addr := host + ":" + port
	log.Printf("starting Book Forge API server on %s", addr)
	if err := r.Run(addr); err != nil {
		log.Fatalf("server failed: %v", err)
	}
}
