package server

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"
)

func TestAuthSessionWhenAuthIsNotConfigured(t *testing.T) {
	state := NewAppState(NewSharedFetcher())
	router := SetupRouter(state)

	rec := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodGet, "/api/auth/session", nil)
	router.ServeHTTP(rec, req)

	if rec.Code != http.StatusOK {
		t.Fatalf("expected 200, got %d", rec.Code)
	}
	var session AuthSessionResponse
	if err := json.Unmarshal(rec.Body.Bytes(), &session); err != nil {
		t.Fatalf("decode session response: %v", err)
	}
	if session.AuthRequired {
		t.Fatalf("expected authRequired=false")
	}
	if !session.Authenticated {
		t.Fatalf("expected authenticated=true for auth-disabled local mode")
	}
}

func TestAuthServiceFromEnvRequiresSessionSecret(t *testing.T) {
	t.Setenv("GITHUB_CLIENT_ID", "client-id")
	t.Setenv("GITHUB_CLIENT_SECRET", "client-secret")
	t.Setenv("AUTH_SESSION_SECRET", "")
	t.Setenv("AUTH_BASE_URL", "http://book.test")

	_, err := NewAuthServiceFromEnv()
	if err == nil || !strings.Contains(err.Error(), "AUTH_SESSION_SECRET") {
		t.Fatalf("expected AUTH_SESSION_SECRET error, got %v", err)
	}
}

func TestAuthServiceFromEnvRequiresBaseURL(t *testing.T) {
	t.Setenv("GITHUB_CLIENT_ID", "client-id")
	t.Setenv("GITHUB_CLIENT_SECRET", "client-secret")
	t.Setenv("AUTH_SESSION_SECRET", "test-session-secret-with-enough-entropy")
	t.Setenv("AUTH_BASE_URL", "")

	_, err := NewAuthServiceFromEnv()
	if err == nil || !strings.Contains(err.Error(), "AUTH_BASE_URL") {
		t.Fatalf("expected AUTH_BASE_URL error, got %v", err)
	}
}

func TestAuthServiceRejectsShortSessionSecret(t *testing.T) {
	_, err := NewAuthService(AuthConfig{
		ClientID:      "client-id",
		ClientSecret:  "client-secret",
		SessionSecret: "short",
		BaseURL:       "http://book.test",
	})
	if err == nil || !strings.Contains(err.Error(), "at least 32 characters") {
		t.Fatalf("expected short secret error, got %v", err)
	}
}

func TestProtectedRoutesRequireSessionWhenAuthIsConfigured(t *testing.T) {
	auth := newTestAuthService(t, AuthConfig{})
	state := NewAppState(NewSharedFetcher())
	state.Auth = auth
	router := SetupRouter(state)

	rec := httptest.NewRecorder()
	req := httptest.NewRequest(http.MethodGet, "/api/preview?url=https%3A%2F%2Fexample.com", nil)
	router.ServeHTTP(rec, req)

	if rec.Code != http.StatusUnauthorized {
		t.Fatalf("expected 401, got %d", rec.Code)
	}
	var payload ErrorPayload
	if err := json.Unmarshal(rec.Body.Bytes(), &payload); err != nil {
		t.Fatalf("decode error response: %v", err)
	}
	if payload.Error.Code != "authentication_required" {
		t.Fatalf("expected authentication_required, got %q", payload.Error.Code)
	}
}

func TestSessionUserRechecksAllowedLogins(t *testing.T) {
	auth := newTestAuthService(t, AuthConfig{AllowedLogins: []string{"octocat"}})
	value, err := auth.encodeSignedJSON(authSessionCookie{
		User:      AuthenticatedUser{ID: 123, Login: "blocked-user"},
		ExpiresAt: time.Now().Add(time.Hour).Unix(),
	})
	if err != nil {
		t.Fatalf("encode session: %v", err)
	}

	req := httptest.NewRequest(http.MethodGet, "/api/auth/session", nil)
	req.AddCookie(&http.Cookie{Name: sessionCookieName, Value: value})
	if _, ok := auth.SessionUser(req); ok {
		t.Fatalf("expected blocked allowlist session")
	}
}

func TestGitHubOAuthLoginCallbackAndSession(t *testing.T) {
	var tokenRequestCode string
	var tokenRequestRedirectURI string
	provider := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/token":
			if r.Method != http.MethodPost {
				t.Fatalf("expected POST token request, got %s", r.Method)
			}
			if err := r.ParseForm(); err != nil {
				t.Fatalf("parse token form: %v", err)
			}
			if r.Form.Get("client_id") != "client-id" {
				t.Fatalf("unexpected client_id %q", r.Form.Get("client_id"))
			}
			if r.Form.Get("client_secret") != "client-secret" {
				t.Fatalf("unexpected client_secret")
			}
			tokenRequestCode = r.Form.Get("code")
			tokenRequestRedirectURI = r.Form.Get("redirect_uri")
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"access_token":"gh-token","token_type":"bearer","scope":"read:user"}`))
		case "/user":
			if got := r.Header.Get("Authorization"); got != "Bearer gh-token" {
				t.Fatalf("unexpected authorization header %q", got)
			}
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"id":123,"login":"octocat","name":"Octo Cat","avatar_url":"https://avatars.example/octo.png"}`))
		default:
			http.NotFound(w, r)
		}
	}))
	defer provider.Close()

	auth := newTestAuthService(t, AuthConfig{
		AuthorizeURL: provider.URL + "/authorize",
		TokenURL:     provider.URL + "/token",
		UserURL:      provider.URL + "/user",
		BaseURL:      "http://book.test",
		HTTPClient:   provider.Client(),
	})
	state := NewAppState(NewSharedFetcher())
	state.Auth = auth
	router := SetupRouter(state)

	loginRec := httptest.NewRecorder()
	loginReq := httptest.NewRequest(http.MethodGet, "/api/auth/login?returnTo=/forge?x=1", nil)
	router.ServeHTTP(loginRec, loginReq)

	if loginRec.Code != http.StatusFound {
		t.Fatalf("expected login redirect, got %d", loginRec.Code)
	}
	location := loginRec.Header().Get("Location")
	redirectURL, err := url.Parse(location)
	if err != nil {
		t.Fatalf("parse login redirect: %v", err)
	}
	if redirectURL.Scheme+"://"+redirectURL.Host+redirectURL.Path != provider.URL+"/authorize" {
		t.Fatalf("unexpected authorize redirect %q", location)
	}
	if redirectURL.Query().Get("client_id") != "client-id" {
		t.Fatalf("unexpected client_id %q", redirectURL.Query().Get("client_id"))
	}
	if redirectURL.Query().Get("redirect_uri") != "http://book.test/api/auth/callback" {
		t.Fatalf("unexpected redirect_uri %q", redirectURL.Query().Get("redirect_uri"))
	}
	oauthState := redirectURL.Query().Get("state")
	if oauthState == "" {
		t.Fatalf("expected oauth state")
	}
	stateCookie := cookieByName(loginRec.Result().Cookies(), stateCookieName)
	if stateCookie == nil {
		t.Fatalf("expected oauth state cookie")
	}
	if !stateCookie.HttpOnly {
		t.Fatalf("expected oauth state cookie to be httpOnly")
	}

	callbackRec := httptest.NewRecorder()
	callbackReq := httptest.NewRequest(http.MethodGet, "/api/auth/callback?code=abc123&state="+url.QueryEscape(oauthState), nil)
	callbackReq.AddCookie(stateCookie)
	router.ServeHTTP(callbackRec, callbackReq)

	if callbackRec.Code != http.StatusSeeOther {
		t.Fatalf("expected callback redirect, got %d: %s", callbackRec.Code, callbackRec.Body.String())
	}
	if callbackRec.Header().Get("Location") != "/forge?x=1" {
		t.Fatalf("unexpected callback location %q", callbackRec.Header().Get("Location"))
	}
	if tokenRequestCode != "abc123" {
		t.Fatalf("unexpected token request code %q", tokenRequestCode)
	}
	if tokenRequestRedirectURI != "http://book.test/api/auth/callback" {
		t.Fatalf("unexpected token redirect_uri %q", tokenRequestRedirectURI)
	}

	sessionCookie := cookieByName(callbackRec.Result().Cookies(), sessionCookieName)
	if sessionCookie == nil {
		t.Fatalf("expected session cookie")
	}
	if !sessionCookie.HttpOnly {
		t.Fatalf("expected session cookie to be httpOnly")
	}

	sessionRec := httptest.NewRecorder()
	sessionReq := httptest.NewRequest(http.MethodGet, "/api/auth/session", nil)
	sessionReq.AddCookie(sessionCookie)
	router.ServeHTTP(sessionRec, sessionReq)

	if sessionRec.Code != http.StatusOK {
		t.Fatalf("expected session 200, got %d", sessionRec.Code)
	}
	var session AuthSessionResponse
	if err := json.Unmarshal(sessionRec.Body.Bytes(), &session); err != nil {
		t.Fatalf("decode session response: %v", err)
	}
	if !session.AuthRequired || !session.Authenticated {
		t.Fatalf("expected authenticated required session, got %+v", session)
	}
	if session.User == nil || session.User.Login != "octocat" || session.User.ID != 123 {
		t.Fatalf("unexpected user %+v", session.User)
	}
}

func TestOAuthCallbackRejectsMismatchedState(t *testing.T) {
	auth := newTestAuthService(t, AuthConfig{})
	state := NewAppState(NewSharedFetcher())
	state.Auth = auth
	router := SetupRouter(state)

	loginRec := httptest.NewRecorder()
	loginReq := httptest.NewRequest(http.MethodGet, "/api/auth/login", nil)
	router.ServeHTTP(loginRec, loginReq)
	stateCookie := cookieByName(loginRec.Result().Cookies(), stateCookieName)
	if stateCookie == nil {
		t.Fatalf("expected oauth state cookie")
	}

	callbackRec := httptest.NewRecorder()
	callbackReq := httptest.NewRequest(http.MethodGet, "/api/auth/callback?code=abc123&state=wrong", nil)
	callbackReq.AddCookie(stateCookie)
	router.ServeHTTP(callbackRec, callbackReq)

	if callbackRec.Code != http.StatusBadRequest {
		t.Fatalf("expected 400, got %d", callbackRec.Code)
	}
	if !strings.Contains(callbackRec.Body.String(), "auth_state_invalid") {
		t.Fatalf("expected auth_state_invalid response, got %s", callbackRec.Body.String())
	}
}

func newTestAuthService(t *testing.T, overrides AuthConfig) *AuthService {
	t.Helper()
	config := AuthConfig{
		ClientID:      "client-id",
		ClientSecret:  "client-secret",
		SessionSecret: "test-session-secret-with-enough-entropy",
		BaseURL:       "http://book.test",
		AuthorizeURL:  defaultGitHubAuthorizeURL,
		TokenURL:      defaultGitHubTokenURL,
		UserURL:       defaultGitHubUserURL,
		HTTPClient:    http.DefaultClient,
		SessionTTL:    time.Hour,
		StateTTL:      time.Minute,
	}
	if overrides.ClientID != "" {
		config.ClientID = overrides.ClientID
	}
	if overrides.ClientSecret != "" {
		config.ClientSecret = overrides.ClientSecret
	}
	if overrides.SessionSecret != "" {
		config.SessionSecret = overrides.SessionSecret
	}
	if overrides.BaseURL != "" {
		config.BaseURL = overrides.BaseURL
	}
	if overrides.AuthorizeURL != "" {
		config.AuthorizeURL = overrides.AuthorizeURL
	}
	if overrides.TokenURL != "" {
		config.TokenURL = overrides.TokenURL
	}
	if overrides.UserURL != "" {
		config.UserURL = overrides.UserURL
	}
	if overrides.HTTPClient != nil {
		config.HTTPClient = overrides.HTTPClient
	}
	if overrides.Scopes != "" {
		config.Scopes = overrides.Scopes
	}
	if len(overrides.AllowedLogins) > 0 {
		config.AllowedLogins = overrides.AllowedLogins
	}
	if overrides.SessionTTL > 0 {
		config.SessionTTL = overrides.SessionTTL
	}
	if overrides.StateTTL > 0 {
		config.StateTTL = overrides.StateTTL
	}

	auth, err := NewAuthService(config)
	if err != nil {
		t.Fatalf("create auth service: %v", err)
	}
	return auth
}

func cookieByName(cookies []*http.Cookie, name string) *http.Cookie {
	for _, cookie := range cookies {
		if cookie.Name == name {
			return cookie
		}
	}
	return nil
}
