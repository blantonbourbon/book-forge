package server

import (
	"context"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
)

const (
	defaultGitHubAuthorizeURL = "https://github.com/login/oauth/authorize"
	defaultGitHubTokenURL     = "https://github.com/login/oauth/access_token"
	defaultGitHubUserURL      = "https://api.github.com/user"

	sessionCookieName = "book_forge_session"
	stateCookieName   = "book_forge_oauth_state"

	defaultSessionTTL      = 7 * 24 * time.Hour
	defaultStateTTL        = 10 * time.Minute
	minSessionSecretLength = 32
)

type AuthConfig struct {
	ClientID      string
	ClientSecret  string
	SessionSecret string
	BaseURL       string
	Scopes        string

	AuthorizeURL string
	TokenURL     string
	UserURL      string
	HTTPClient   *http.Client

	AllowedLogins []string
	SessionTTL    time.Duration
	StateTTL      time.Duration
}

type AuthService struct {
	clientID      string
	clientSecret  string
	sessionSecret []byte
	baseURL       string
	scopes        string
	authorizeURL  string
	tokenURL      string
	userURL       string
	httpClient    *http.Client
	allowedLogins map[string]struct{}
	sessionTTL    time.Duration
	stateTTL      time.Duration
}

type AuthenticatedUser struct {
	ID        int64  `json:"id"`
	Login     string `json:"login"`
	Name      string `json:"name,omitempty"`
	AvatarURL string `json:"avatarUrl,omitempty"`
	Email     string `json:"email,omitempty"`
}

type AuthSessionResponse struct {
	AuthRequired  bool               `json:"authRequired"`
	Authenticated bool               `json:"authenticated"`
	User          *AuthenticatedUser `json:"user,omitempty"`
	LoginURL      string             `json:"loginUrl,omitempty"`
	LogoutURL     string             `json:"logoutUrl,omitempty"`
}

type oauthStateCookie struct {
	State     string `json:"state"`
	ReturnTo  string `json:"returnTo"`
	ExpiresAt int64  `json:"expiresAt"`
}

type authSessionCookie struct {
	User      AuthenticatedUser `json:"user"`
	ExpiresAt int64             `json:"expiresAt"`
}

type githubTokenResponse struct {
	AccessToken      string `json:"access_token"`
	TokenType        string `json:"token_type"`
	Scope            string `json:"scope"`
	Error            string `json:"error"`
	ErrorDescription string `json:"error_description"`
}

type githubUserResponse struct {
	ID        int64  `json:"id"`
	Login     string `json:"login"`
	Name      string `json:"name"`
	AvatarURL string `json:"avatar_url"`
	Email     string `json:"email"`
	Message   string `json:"message"`
}

func NewAuthServiceFromEnv() (*AuthService, error) {
	clientID := strings.TrimSpace(os.Getenv("GITHUB_CLIENT_ID"))
	clientSecret := strings.TrimSpace(os.Getenv("GITHUB_CLIENT_SECRET"))
	if clientID == "" && clientSecret == "" {
		return nil, nil
	}
	if clientID == "" || clientSecret == "" {
		return nil, fmt.Errorf("GITHUB_CLIENT_ID and GITHUB_CLIENT_SECRET must be set together")
	}

	sessionSecret := os.Getenv("AUTH_SESSION_SECRET")
	if strings.TrimSpace(sessionSecret) == "" {
		return nil, fmt.Errorf("AUTH_SESSION_SECRET must be set when GitHub sign-in is configured")
	}
	baseURL := strings.TrimSpace(os.Getenv("AUTH_BASE_URL"))
	if baseURL == "" {
		return nil, fmt.Errorf("AUTH_BASE_URL must be set when GitHub sign-in is configured")
	}

	return NewAuthService(AuthConfig{
		ClientID:      clientID,
		ClientSecret:  clientSecret,
		SessionSecret: sessionSecret,
		BaseURL:       baseURL,
		Scopes:        firstConfigured(strings.TrimSpace(os.Getenv("GITHUB_OAUTH_SCOPES")), "read:user"),
		AllowedLogins: splitCSV(os.Getenv("AUTH_ALLOWED_GITHUB_LOGINS")),
		AuthorizeURL:  defaultGitHubAuthorizeURL,
		TokenURL:      defaultGitHubTokenURL,
		UserURL:       defaultGitHubUserURL,
		HTTPClient:    &http.Client{Timeout: 15 * time.Second},
		SessionTTL:    defaultSessionTTL,
		StateTTL:      defaultStateTTL,
	})
}

func NewAuthService(config AuthConfig) (*AuthService, error) {
	if strings.TrimSpace(config.ClientID) == "" {
		return nil, fmt.Errorf("auth client id is required")
	}
	if strings.TrimSpace(config.ClientSecret) == "" {
		return nil, fmt.Errorf("auth client secret is required")
	}
	sessionSecret := strings.TrimSpace(config.SessionSecret)
	if sessionSecret == "" {
		return nil, fmt.Errorf("auth session secret is required")
	}
	if len(sessionSecret) < minSessionSecretLength {
		return nil, fmt.Errorf("auth session secret must be at least %d characters", minSessionSecretLength)
	}

	authorizeURL := firstConfigured(strings.TrimSpace(config.AuthorizeURL), defaultGitHubAuthorizeURL)
	tokenURL := firstConfigured(strings.TrimSpace(config.TokenURL), defaultGitHubTokenURL)
	userURL := firstConfigured(strings.TrimSpace(config.UserURL), defaultGitHubUserURL)
	if _, err := url.ParseRequestURI(authorizeURL); err != nil {
		return nil, fmt.Errorf("auth authorize URL is invalid")
	}
	if _, err := url.ParseRequestURI(tokenURL); err != nil {
		return nil, fmt.Errorf("auth token URL is invalid")
	}
	if _, err := url.ParseRequestURI(userURL); err != nil {
		return nil, fmt.Errorf("auth user URL is invalid")
	}

	baseURL := strings.TrimRight(strings.TrimSpace(config.BaseURL), "/")
	if baseURL != "" {
		parsed, err := url.Parse(baseURL)
		if err != nil || parsed.Scheme == "" || parsed.Host == "" || (parsed.Scheme != "http" && parsed.Scheme != "https") {
			return nil, fmt.Errorf("AUTH_BASE_URL must be an absolute HTTP or HTTPS URL")
		}
	}

	sessionTTL := config.SessionTTL
	if sessionTTL <= 0 {
		sessionTTL = defaultSessionTTL
	}
	stateTTL := config.StateTTL
	if stateTTL <= 0 {
		stateTTL = defaultStateTTL
	}

	httpClient := config.HTTPClient
	if httpClient == nil {
		httpClient = http.DefaultClient
	}

	allowedLogins := make(map[string]struct{})
	for _, login := range config.AllowedLogins {
		normalized := strings.ToLower(strings.TrimSpace(login))
		if normalized != "" {
			allowedLogins[normalized] = struct{}{}
		}
	}

	return &AuthService{
		clientID:      strings.TrimSpace(config.ClientID),
		clientSecret:  config.ClientSecret,
		sessionSecret: []byte(sessionSecret),
		baseURL:       baseURL,
		scopes:        strings.TrimSpace(config.Scopes),
		authorizeURL:  authorizeURL,
		tokenURL:      tokenURL,
		userURL:       userURL,
		httpClient:    httpClient,
		allowedLogins: allowedLogins,
		sessionTTL:    sessionTTL,
		stateTTL:      stateTTL,
	}, nil
}

func (a *AuthService) RequireAuth() gin.HandlerFunc {
	return func(c *gin.Context) {
		user, ok := a.SessionUser(c.Request)
		if !ok {
			RespondError(c, NewAPIError(http.StatusUnauthorized, "authentication_required", "Sign in with GitHub to use Book Forge."))
			return
		}
		c.Set("auth.user", user)
		c.Next()
	}
}

func (a *AuthService) SessionUser(r *http.Request) (*AuthenticatedUser, bool) {
	cookie, err := r.Cookie(sessionCookieName)
	if err != nil || cookie.Value == "" {
		return nil, false
	}

	var session authSessionCookie
	if err := a.decodeSignedJSON(cookie.Value, &session); err != nil {
		return nil, false
	}
	if time.Now().Unix() >= session.ExpiresAt || session.User.ID == 0 || session.User.Login == "" || !a.loginAllowed(session.User.Login) {
		return nil, false
	}

	user := session.User
	return &user, true
}

func (a *AuthService) HandleLogin(c *gin.Context) {
	state, err := randomURLToken(32)
	if err != nil {
		RespondError(c, NewAPIError(http.StatusInternalServerError, "auth_state_failed", "The sign-in request could not be started."))
		return
	}

	returnTo := safeReturnTo(c.Query("returnTo"))
	stateCookie := oauthStateCookie{
		State:     state,
		ReturnTo:  returnTo,
		ExpiresAt: time.Now().Add(a.stateTTL).Unix(),
	}
	value, err := a.encodeSignedJSON(stateCookie)
	if err != nil {
		RespondError(c, NewAPIError(http.StatusInternalServerError, "auth_state_failed", "The sign-in request could not be started."))
		return
	}
	a.setCookie(c, stateCookieName, value, time.Now().Add(a.stateTTL), int(a.stateTTL.Seconds()))

	params := url.Values{}
	params.Set("client_id", a.clientID)
	params.Set("redirect_uri", a.redirectURI(c.Request))
	params.Set("state", state)
	if a.scopes != "" {
		params.Set("scope", a.scopes)
	}

	authorizeURL := a.authorizeURL
	separator := "?"
	if strings.Contains(authorizeURL, "?") {
		separator = "&"
	}
	c.Redirect(http.StatusFound, authorizeURL+separator+params.Encode())
}

func (a *AuthService) HandleCallback(c *gin.Context) {
	if errCode := strings.TrimSpace(c.Query("error")); errCode != "" {
		RespondError(c, BadRequestError("auth_denied", "GitHub sign-in was not completed."))
		return
	}

	code := strings.TrimSpace(c.Query("code"))
	returnedState := strings.TrimSpace(c.Query("state"))
	if code == "" || returnedState == "" {
		RespondError(c, BadRequestError("auth_callback_invalid", "GitHub sign-in returned an invalid callback."))
		return
	}

	state, err := a.readOAuthState(c.Request)
	a.clearCookie(c, stateCookieName)
	if err != nil || state.State != returnedState || time.Now().Unix() >= state.ExpiresAt {
		RespondError(c, BadRequestError("auth_state_invalid", "GitHub sign-in state could not be verified."))
		return
	}

	accessToken, err := a.exchangeCode(c.Request.Context(), code, a.redirectURI(c.Request))
	if err != nil {
		RespondError(c, NewAPIError(http.StatusBadGateway, "auth_token_exchange_failed", "GitHub sign-in could not be completed."))
		return
	}

	user, err := a.fetchUser(c.Request.Context(), accessToken)
	if err != nil {
		RespondError(c, NewAPIError(http.StatusBadGateway, "auth_user_fetch_failed", "GitHub user details could not be fetched."))
		return
	}
	if !a.loginAllowed(user.Login) {
		RespondError(c, NewAPIError(http.StatusForbidden, "auth_user_not_allowed", "This GitHub account is not allowed to use Book Forge."))
		return
	}

	if err := a.setSession(c, *user); err != nil {
		RespondError(c, NewAPIError(http.StatusInternalServerError, "auth_session_failed", "The sign-in session could not be created."))
		return
	}

	c.Redirect(http.StatusSeeOther, safeReturnTo(state.ReturnTo))
}

func (a *AuthService) HandleLogout(c *gin.Context) {
	a.clearCookie(c, sessionCookieName)
	c.JSON(http.StatusOK, AuthSessionResponse{
		AuthRequired:  true,
		Authenticated: false,
		LoginURL:      "/api/auth/login",
		LogoutURL:     "/api/auth/logout",
	})
}

func (a *AuthService) readOAuthState(r *http.Request) (oauthStateCookie, error) {
	cookie, err := r.Cookie(stateCookieName)
	if err != nil || cookie.Value == "" {
		return oauthStateCookie{}, err
	}

	var state oauthStateCookie
	if err := a.decodeSignedJSON(cookie.Value, &state); err != nil {
		return oauthStateCookie{}, err
	}
	return state, nil
}

func (a *AuthService) exchangeCode(ctx context.Context, code, redirectURI string) (string, error) {
	form := url.Values{}
	form.Set("client_id", a.clientID)
	form.Set("client_secret", a.clientSecret)
	form.Set("code", code)
	form.Set("redirect_uri", redirectURI)

	req, err := http.NewRequestWithContext(ctx, http.MethodPost, a.tokenURL, strings.NewReader(form.Encode()))
	if err != nil {
		return "", err
	}
	req.Header.Set("Accept", "application/json")
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")

	resp, err := a.httpClient.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if err != nil {
		return "", err
	}

	var token githubTokenResponse
	if err := json.Unmarshal(body, &token); err != nil {
		return "", err
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 || token.Error != "" || token.AccessToken == "" {
		return "", errors.New("github token exchange failed")
	}
	return token.AccessToken, nil
}

func (a *AuthService) fetchUser(ctx context.Context, accessToken string) (*AuthenticatedUser, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, a.userURL, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Accept", "application/vnd.github+json")
	req.Header.Set("Authorization", "Bearer "+accessToken)
	req.Header.Set("X-GitHub-Api-Version", "2022-11-28")

	resp, err := a.httpClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if err != nil {
		return nil, err
	}

	var ghUser githubUserResponse
	if err := json.Unmarshal(body, &ghUser); err != nil {
		return nil, err
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 || ghUser.ID == 0 || ghUser.Login == "" {
		return nil, errors.New("github user fetch failed")
	}

	return &AuthenticatedUser{
		ID:        ghUser.ID,
		Login:     ghUser.Login,
		Name:      ghUser.Name,
		AvatarURL: ghUser.AvatarURL,
		Email:     ghUser.Email,
	}, nil
}

func (a *AuthService) setSession(c *gin.Context, user AuthenticatedUser) error {
	expires := time.Now().Add(a.sessionTTL)
	value, err := a.encodeSignedJSON(authSessionCookie{
		User:      user,
		ExpiresAt: expires.Unix(),
	})
	if err != nil {
		return err
	}
	a.setCookie(c, sessionCookieName, value, expires, int(a.sessionTTL.Seconds()))
	return nil
}

func (a *AuthService) loginAllowed(login string) bool {
	if len(a.allowedLogins) == 0 {
		return true
	}
	_, ok := a.allowedLogins[strings.ToLower(strings.TrimSpace(login))]
	return ok
}

func (a *AuthService) redirectURI(r *http.Request) string {
	base := a.baseURL
	if base == "" {
		base = requestBaseURL(r)
	}
	return strings.TrimRight(base, "/") + "/api/auth/callback"
}

func (a *AuthService) encodeSignedJSON(value any) (string, error) {
	payload, err := json.Marshal(value)
	if err != nil {
		return "", err
	}
	encoded := base64.RawURLEncoding.EncodeToString(payload)
	signature := a.sign(encoded)
	return encoded + "." + signature, nil
}

func (a *AuthService) decodeSignedJSON(value string, target any) error {
	encoded, signature, ok := strings.Cut(value, ".")
	if !ok || encoded == "" || signature == "" {
		return fmt.Errorf("invalid signed value")
	}
	expected := a.sign(encoded)
	if !hmac.Equal([]byte(signature), []byte(expected)) {
		return fmt.Errorf("invalid signed value")
	}

	payload, err := base64.RawURLEncoding.DecodeString(encoded)
	if err != nil {
		return err
	}
	return json.Unmarshal(payload, target)
}

func (a *AuthService) sign(value string) string {
	mac := hmac.New(sha256.New, a.sessionSecret)
	mac.Write([]byte(value))
	return base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
}

func (a *AuthService) setCookie(c *gin.Context, name, value string, expires time.Time, maxAge int) {
	http.SetCookie(c.Writer, &http.Cookie{
		Name:     name,
		Value:    value,
		Path:     "/",
		MaxAge:   maxAge,
		Expires:  expires,
		HttpOnly: true,
		Secure:   requestIsSecure(c.Request, a.baseURL),
		SameSite: http.SameSiteLaxMode,
	})
}

func (a *AuthService) clearCookie(c *gin.Context, name string) {
	http.SetCookie(c.Writer, &http.Cookie{
		Name:     name,
		Value:    "",
		Path:     "/",
		MaxAge:   -1,
		Expires:  time.Unix(0, 0),
		HttpOnly: true,
		Secure:   requestIsSecure(c.Request, a.baseURL),
		SameSite: http.SameSiteLaxMode,
	})
}

func requestBaseURL(r *http.Request) string {
	proto := firstForwardedValue(r.Header.Get("X-Forwarded-Proto"))
	if proto == "" {
		if r.TLS != nil {
			proto = "https"
		} else {
			proto = "http"
		}
	}
	host := firstForwardedValue(r.Header.Get("X-Forwarded-Host"))
	if host == "" {
		host = r.Host
	}
	return proto + "://" + host
}

func requestIsSecure(r *http.Request, baseURL string) bool {
	if parsed, err := url.Parse(baseURL); err == nil && parsed.Scheme == "https" {
		return true
	}
	if strings.EqualFold(firstForwardedValue(r.Header.Get("X-Forwarded-Proto")), "https") {
		return true
	}
	return r.TLS != nil
}

func safeReturnTo(raw string) string {
	if raw == "" {
		return "/"
	}
	if strings.Contains(raw, `\`) || strings.HasPrefix(raw, "//") || !strings.HasPrefix(raw, "/") {
		return "/"
	}
	parsed, err := url.Parse(raw)
	if err != nil || parsed.IsAbs() || parsed.Host != "" {
		return "/"
	}
	return raw
}

func randomURLToken(byteCount int) (string, error) {
	bytes := make([]byte, byteCount)
	if _, err := rand.Read(bytes); err != nil {
		return "", err
	}
	return base64.RawURLEncoding.EncodeToString(bytes), nil
}

func splitCSV(value string) []string {
	if value == "" {
		return nil
	}
	parts := strings.Split(value, ",")
	result := make([]string, 0, len(parts))
	for _, part := range parts {
		trimmed := strings.TrimSpace(part)
		if trimmed != "" {
			result = append(result, trimmed)
		}
	}
	return result
}

func firstForwardedValue(value string) string {
	if value == "" {
		return ""
	}
	first, _, _ := strings.Cut(value, ",")
	return strings.TrimSpace(first)
}

func firstConfigured(values ...string) string {
	for _, value := range values {
		if strings.TrimSpace(value) != "" {
			return value
		}
	}
	return ""
}
