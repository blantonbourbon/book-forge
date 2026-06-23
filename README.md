# Book Forge

Book Forge converts public HTML pages or same-site crawls into downloadable EPUB files. It uses a Go/Gin backend and a static Astro frontend served from the same origin in production.

## Requirements

- Node.js 24+
- Go 1.25+
- Docker for container builds
- Fly CLI for manual deployment

## Local setup

```bash
npm run validate:install
```

Run the backend with the built frontend:

```bash
npm run build --prefix frontend
HOST=127.0.0.1 PORT=3100 STATIC_DIR=frontend/dist go run .
```

## GitHub sign-in

When `GITHUB_CLIENT_ID` and `GITHUB_CLIENT_SECRET` are set, Book Forge requires a signed GitHub session for conversion API routes. Register the OAuth callback URL as:

```text
http://127.0.0.1:3100/api/auth/callback
```

For deployment, use the public origin instead, for example:

```text
https://<your-app-host>/api/auth/callback
```

Local example:

```bash
GITHUB_CLIENT_ID=<client-id> \
GITHUB_CLIENT_SECRET=<client-secret> \
AUTH_SESSION_SECRET=<random-32-plus-character-secret> \
AUTH_BASE_URL=http://127.0.0.1:3100 \
HOST=127.0.0.1 PORT=3100 STATIC_DIR=frontend/dist go run .
```

`AUTH_SESSION_SECRET` and `AUTH_BASE_URL` are required when GitHub sign-in is enabled.

Optional settings:

- `AUTH_ALLOWED_GITHUB_LOGINS`: comma-separated GitHub usernames allowed to sign in. If unset, any GitHub account that authorizes the app can use it.
- `GITHUB_OAUTH_SCOPES`: GitHub OAuth scopes. Defaults to `read:user`.

For frontend development, start Astro on port `3101` while the backend runs on `3100`:

```bash
npm run dev --prefix frontend -- --host 127.0.0.1 --port 3101
```

## Validation

```bash
npm run format
npm run lint
npm run typecheck
npm test
npm run build
npm audit --prefix frontend --omit=dev --audit-level=moderate
```

## Container

```bash
docker build -t book-forge:local .
docker run --rm -p 3100:3100 book-forge:local
curl -fsS http://127.0.0.1:3100/api/health
```

## Fly.io deployment

The app name is `book-forge` and the internal port is `3100`.

1. Create a GitHub Actions repository secret named `FLY_API_TOKEN`.
2. Push to `main`.
3. The CI workflow validates the repo, then deploys to Fly only on `main` pushes when the secret is present.

Manual deploy:

```bash
flyctl deploy --remote-only --config fly.toml
```
