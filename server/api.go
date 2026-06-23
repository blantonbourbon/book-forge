package server

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"

	"github.com/gin-gonic/gin"
	"github.com/google/uuid"
)

const maxJobRequestBytes = 1 << 20 // 1 MiB

type HealthResponse struct {
	Status string `json:"status"`
}

func SetupRouter(state *AppState) *gin.Engine {
	r := gin.New()
	r.Use(gin.Logger(), gin.Recovery())

	api := r.Group("/api")
	{
		api.GET("/health", handleHealth)
		auth := api.Group("/auth")
		{
			auth.GET("/session", handleAuthSession(state))
			auth.GET("/login", handleAuthLogin(state))
			auth.GET("/callback", handleAuthCallback(state))
			auth.POST("/logout", handleAuthLogout(state))
		}

		protected := api.Group("")
		if state.Auth != nil {
			protected.Use(state.Auth.RequireAuth())
		}
		protected.GET("/preview", handlePreviewMetadata(state))
		protected.POST("/jobs", handleCreateJob(state))
		protected.GET("/jobs/:id", handleGetJob(state))
		protected.GET("/jobs/:id/download", handleDownload(state))
	}

	r.NoRoute(handleStatic(state))
	return r
}

func handleHealth(c *gin.Context) {
	if c.Request.Method != http.MethodGet {
		RespondError(c, MethodNotAllowedError())
		return
	}
	c.JSON(http.StatusOK, HealthResponse{Status: "healthy"})
}

func handleAuthSession(state *AppState) gin.HandlerFunc {
	return func(c *gin.Context) {
		if state.Auth == nil {
			c.JSON(http.StatusOK, AuthSessionResponse{
				AuthRequired:  false,
				Authenticated: true,
			})
			return
		}

		user, ok := state.Auth.SessionUser(c.Request)
		response := AuthSessionResponse{
			AuthRequired:  true,
			Authenticated: ok,
			LoginURL:      "/api/auth/login",
			LogoutURL:     "/api/auth/logout",
		}
		if ok {
			response.User = user
		}
		c.JSON(http.StatusOK, response)
	}
}

func handleAuthLogin(state *AppState) gin.HandlerFunc {
	return func(c *gin.Context) {
		if state.Auth == nil {
			RespondError(c, NotFoundError("auth_not_configured", "GitHub sign-in is not configured."))
			return
		}
		state.Auth.HandleLogin(c)
	}
}

func handleAuthCallback(state *AppState) gin.HandlerFunc {
	return func(c *gin.Context) {
		if state.Auth == nil {
			RespondError(c, NotFoundError("auth_not_configured", "GitHub sign-in is not configured."))
			return
		}
		state.Auth.HandleCallback(c)
	}
}

func handleAuthLogout(state *AppState) gin.HandlerFunc {
	return func(c *gin.Context) {
		if state.Auth == nil {
			c.JSON(http.StatusOK, AuthSessionResponse{
				AuthRequired:  false,
				Authenticated: true,
			})
			return
		}
		state.Auth.HandleLogout(c)
	}
}

func handleCreateJob(state *AppState) gin.HandlerFunc {
	return func(c *gin.Context) {
		if c.Request.Method != http.MethodPost {
			RespondError(c, MethodNotAllowedError())
			return
		}

		c.Request.Body = http.MaxBytesReader(c.Writer, c.Request.Body, maxJobRequestBytes)
		body, err := io.ReadAll(c.Request.Body)
		if err != nil {
			var maxErr *http.MaxBytesError
			if errors.As(err, &maxErr) {
				RespondError(c, NewAPIError(http.StatusRequestEntityTooLarge, "request_too_large", "Job request body exceeded the configured limit."))
				return
			}
			RespondError(c, ValidationError("Job creation requires a JSON request body.", []string{"body"}))
			return
		}
		if len(body) == 0 {
			RespondError(c, ValidationError("Job creation requires a JSON request body.", []string{"body"}))
			return
		}

		var req CreateJobRequest
		if err := json.Unmarshal(body, &req); err != nil {
			RespondError(c, ValidationError("Job creation requires valid JSON fields.", []string{"body"}))
			return
		}

		summary, err := ValidateCreateRequest(req)
		if err != nil {
			if apiErr, ok := err.(*APIError); ok {
				RespondError(c, apiErr)
			} else {
				RespondError(c, ValidationError(err.Error(), nil))
			}
			return
		}

		if err := EnforceCreateRequestSecurity(summary); err != nil {
			if apiErr, ok := err.(*APIError); ok {
				RespondError(c, apiErr)
			} else {
				RespondError(c, ValidationError(err.Error(), nil))
			}
			return
		}

		resp, err := state.Jobs.CreateJob(state.Fetcher, state.BrowserFetcher, *summary)
		if err != nil {
			RespondError(c, NewAPIError(http.StatusInternalServerError, "job_creation_failed", err.Error()))
			return
		}

		c.JSON(http.StatusAccepted, resp)
	}
}

func handleGetJob(state *AppState) gin.HandlerFunc {
	return func(c *gin.Context) {
		if c.Request.Method != http.MethodGet {
			RespondError(c, MethodNotAllowedError())
			return
		}

		idStr := c.Param("id")
		id, err := uuid.Parse(idStr)
		if err != nil {
			RespondError(c, BadRequestError("invalid_job_id", "The job id is not valid."))
			return
		}

		resp, err := state.Jobs.GetResponse(id)
		if err != nil {
			if apiErr, ok := err.(*APIError); ok {
				RespondError(c, apiErr)
			} else {
				RespondError(c, NotFoundError("job_not_found", "The requested job was not found."))
			}
			return
		}

		c.JSON(http.StatusOK, resp)
	}
}

func handleDownload(state *AppState) gin.HandlerFunc {
	return func(c *gin.Context) {
		if c.Request.Method != http.MethodGet {
			RespondError(c, MethodNotAllowedError())
			return
		}

		idStr := c.Param("id")
		id, err := uuid.Parse(idStr)
		if err != nil {
			RespondError(c, BadRequestError("invalid_job_id", "The job id is not valid."))
			return
		}

		status, artifact := state.Jobs.Artifact(id)
		if status == "" {
			RespondError(c, NotFoundError("job_not_found", "The requested job was not found."))
			return
		}

		switch status {
		case StatusCompleted:
			if artifact == nil {
				RespondError(c, ConflictError("download_unavailable", "The completed EPUB artifact is not available."))
				return
			}
			filename := safeHeaderFilename(artifact.Filename)
			c.Header("Content-Type", "application/epub+zip")
			c.Header("Content-Length", fmt.Sprintf("%d", len(artifact.Bytes)))
			c.Header("Content-Disposition", fmt.Sprintf(`attachment; filename="%s"`, filename))
			c.Data(http.StatusOK, "application/epub+zip", artifact.Bytes)
		case StatusFailed:
			RespondError(c, ConflictError("job_failed", "Failed jobs do not have downloadable EPUB artifacts."))
		default:
			RespondError(c, ConflictError("job_not_completed", "The EPUB download is available only after the job completes."))
		}
	}
}

func handleStatic(state *AppState) gin.HandlerFunc {
	return func(c *gin.Context) {
		if c.Request.Method != http.MethodGet && c.Request.Method != http.MethodHead {
			RespondError(c, MethodNotAllowedError())
			return
		}

		if state.StaticRoot == "" {
			RespondError(c, NotFoundError("route_not_found", "The requested route was not found."))
			return
		}

		requestPath := c.Request.URL.Path
		target, err := staticTarget(state.StaticRoot, requestPath)
		if err != nil {
			if apiErr, ok := err.(*APIError); ok {
				RespondError(c, apiErr)
			} else {
				RespondError(c, NotFoundError("static_asset_not_found", "The requested asset was not found."))
			}
			return
		}

		bytes, err := os.ReadFile(target)
		if err != nil {
			RespondError(c, NotFoundError("static_asset_not_found", "The requested asset was not found."))
			return
		}

		if c.Request.Method == http.MethodHead {
			c.Header("Content-Type", contentTypes(target))
			c.Header("Content-Length", fmt.Sprintf("%d", len(bytes)))
			c.Status(http.StatusOK)
			return
		}

		c.Data(http.StatusOK, contentTypes(target), bytes)
	}
}

func staticTarget(root, requestPath string) (string, error) {
	if err := rejectSuspiciousPath(requestPath); err != nil {
		return "", err
	}

	relative := strings.TrimPrefix(requestPath, "/")
	clean := filepath.Join(root, relative)

	if info, err := os.Stat(clean); err == nil && !info.IsDir() {
		return clean, nil
	}

	if shouldFallbackToIndex(relative) {
		indexPath := filepath.Join(root, "index.html")
		if _, err := os.Stat(indexPath); err == nil {
			return indexPath, nil
		}
	}

	return "", NotFoundError("static_asset_not_found", "The requested asset was not found.")
}

func rejectSuspiciousPath(requestPath string) error {
	lower := strings.ToLower(requestPath)
	if strings.Contains(lower, "..") ||
		strings.Contains(lower, "%2e") ||
		strings.Contains(lower, "%2f") ||
		strings.Contains(lower, "%5c") ||
		strings.Contains(lower, `\`) {
		return BadRequestError("static_path_rejected", "The requested static path was not accepted.")
	}
	return nil
}

func shouldFallbackToIndex(relative string) bool {
	if relative == "" {
		return true
	}
	lastSlash := strings.LastIndexByte(relative, '/')
	last := relative[lastSlash+1:]
	return !strings.Contains(last, ".")
}

func safeHeaderFilename(filename string) string {
	var safe strings.Builder
	for _, ch := range filename {
		if (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') || (ch >= '0' && ch <= '9') || ch == ' ' || ch == '.' || ch == '-' || ch == '_' {
			safe.WriteRune(ch)
		} else if !strings.HasSuffix(safe.String(), "_") {
			safe.WriteByte('_')
		}
	}
	result := safe.String()
	for strings.Contains(result, "..") {
		result = strings.ReplaceAll(result, "..", ".")
	}
	result = strings.Trim(result, " .")
	if result == "" {
		result = "book-forge.epub"
	}
	if !strings.HasSuffix(strings.ToLower(result), ".epub") {
		result += ".epub"
	}
	return result
}

func contentTypes(path string) string {
	ext := strings.ToLower(filepath.Ext(path))
	switch ext {
	case ".html", ".htm":
		return "text/html; charset=utf-8"
	case ".css":
		return "text/css; charset=utf-8"
	case ".js", ".mjs":
		return "text/javascript; charset=utf-8"
	case ".json":
		return "application/json"
	case ".svg":
		return "image/svg+xml"
	case ".png":
		return "image/png"
	case ".jpg", ".jpeg":
		return "image/jpeg"
	case ".webp":
		return "image/webp"
	case ".ico":
		return "image/x-icon"
	case ".txt":
		return "text/plain; charset=utf-8"
	default:
		return "application/octet-stream"
	}
}
