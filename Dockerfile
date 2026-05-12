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

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates \
  && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --shell /usr/sbin/nologin bookforge
WORKDIR /app
COPY --from=backend-builder /usr/local/bin/book-forge-server /usr/local/bin/book-forge-server
COPY --from=frontend-builder /app/frontend/dist ./frontend
RUN chown -R bookforge:bookforge /app

USER bookforge
ENV HOST=0.0.0.0
ENV PORT=3100
ENV STATIC_DIR=/app/frontend
EXPOSE 3100

CMD ["/usr/local/bin/book-forge-server"]
