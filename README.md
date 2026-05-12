# Book Forge

Book Forge converts public HTML pages or same-site crawls into downloadable EPUB files. It uses a Rust/Axum backend and a static Astro frontend served from the same origin in production.

## Requirements

- Node.js 24+
- Rust stable with `cargo`, `rustfmt`, and `clippy`
- Docker for container builds
- Fly CLI for manual deployment

## Local setup

```bash
npm run validate:install
```

Run the backend with the built frontend:

```bash
npm run build --prefix frontend
HOST=127.0.0.1 PORT=3100 STATIC_DIR=frontend/dist cargo run --locked --bin book-forge-server
```

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
2. Push to `master`.
3. The CI workflow validates the repo, then deploys to Fly only on `master` pushes when the secret is present.

Manual deploy:

```bash
flyctl deploy --remote-only --config fly.toml
```
