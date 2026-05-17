FROM node:24-bookworm-slim AS frontend-builder
WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

FROM golang:1.25-bookworm AS backend-builder
WORKDIR /app
COPY go.mod go.sum ./
RUN go mod download
COPY . ./
RUN CGO_ENABLED=0 go build -o /usr/local/bin/book-forge-server .

FROM node:24-bookworm-slim AS sidecar-builder
ENV CLOAKBROWSER_AUTO_UPDATE=false
WORKDIR /app/cloak-sidecar
COPY cloak-sidecar/package.json cloak-sidecar/package-lock.json ./
RUN npm ci
COPY cloak-sidecar/ ./
RUN npx --yes cloakbrowser install

FROM node:24-bookworm-slim AS runtime
RUN apt-get update \
  && apt-get install -y --no-install-recommends \
       bash \
       ca-certificates \
       fonts-liberation \
       libasound2 \
       libatk-bridge2.0-0 \
       libatk1.0-0 \
       libatspi2.0-0 \
       libcairo2 \
       libcups2 \
       libdbus-1-3 \
       libdrm2 \
       libgbm1 \
       libnspr4 \
       libnss3 \
       libpango-1.0-0 \
       libwayland-client0 \
       libxcomposite1 \
       libxdamage1 \
       libxfixes3 \
       libxkbcommon0 \
       libxrandr2 \
  && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --shell /usr/sbin/nologin bookforge

WORKDIR /app
COPY --from=backend-builder /usr/local/bin/book-forge-server /usr/local/bin/book-forge-server
COPY --from=frontend-builder --chown=bookforge:bookforge /app/frontend/dist ./frontend
COPY --from=sidecar-builder --chown=bookforge:bookforge /app/cloak-sidecar ./cloak-sidecar
COPY --from=sidecar-builder --chown=bookforge:bookforge /root/.cloakbrowser /home/bookforge/.cloakbrowser
COPY --chmod=0755 scripts/entrypoint.sh /usr/local/bin/entrypoint.sh

USER bookforge
ENV HOME=/home/bookforge
ENV HOST=0.0.0.0
ENV PORT=3100
ENV STATIC_DIR=/app/frontend
ENV CLOAK_SIDECAR_URL=http://127.0.0.1:3102
ENV CLOAK_PORT=3102
ENV CLOAKBROWSER_AUTO_UPDATE=false
EXPOSE 3100

CMD ["/usr/local/bin/entrypoint.sh"]
