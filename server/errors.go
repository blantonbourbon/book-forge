package server

import (
	"net/http"
	"strings"

	"github.com/gin-gonic/gin"
)

type ErrorPayload struct {
	Error ErrorBody `json:"error"`
}

type ErrorBody struct {
	Code    string   `json:"code"`
	Message string   `json:"message"`
	Fields  []string `json:"fields,omitempty"`
}

type APIError struct {
	Status int
	Body   ErrorBody
}

func (e *APIError) Error() string {
	return e.Body.Message
}

func NewAPIError(status int, code, message string) *APIError {
	return &APIError{
		Status: status,
		Body: ErrorBody{
			Code:    code,
			Message: sanitizeMessage(message),
		},
	}
}

func ValidationError(message string, fields []string) *APIError {
	return &APIError{
		Status: http.StatusUnprocessableEntity,
		Body: ErrorBody{
			Code:    "validation_failed",
			Message: sanitizeMessage(message),
			Fields:  fields,
		},
	}
}

func BadRequestError(code, message string) *APIError {
	return NewAPIError(http.StatusBadRequest, code, message)
}

func NotFoundError(code, message string) *APIError {
	return NewAPIError(http.StatusNotFound, code, message)
}

func ConflictError(code, message string) *APIError {
	return NewAPIError(http.StatusConflict, code, message)
}

func MethodNotAllowedError() *APIError {
	return NewAPIError(http.StatusMethodNotAllowed, "method_not_allowed", "This HTTP method is not supported for the requested API route.")
}

func RespondError(c *gin.Context, err *APIError) {
	c.AbortWithStatusJSON(err.Status, ErrorPayload{Error: err.Body})
}

func sanitizeMessage(message string) string {
	var sanitized strings.Builder
	for _, ch := range message {
		if ch < 32 {
			sanitized.WriteByte(' ')
		} else {
			sanitized.WriteRune(ch)
		}
	}
	result := strings.Join(strings.Fields(sanitized.String()), " ")
	if result == "" || containsSensitiveMarker(result) {
		return "The request could not be completed safely."
	}
	return result
}

func containsSensitiveMarker(message string) bool {
	lower := strings.ToLower(message)
	markers := []string{"/home/", `\users\`, "target/debug", "backtrace", "panic"}
	for _, marker := range markers {
		if strings.Contains(lower, marker) {
			return true
		}
	}
	return false
}
