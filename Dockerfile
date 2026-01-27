# Builder stage
FROM rust:alpine AS builder

# Install build dependencies
RUN apk add --no-cache musl-dev

# Create a new empty shell project
WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock ./

# Copy source code
COPY src ./src
COPY tests ./tests

# Build release binary
RUN cargo build --release --locked

# Runtime stage - use minimal alpine image
FROM alpine:latest

# Install runtime dependencies
# ca-certificates needed for HTTPS requests (rustls needs root certificates)
RUN apk add --no-cache ca-certificates tzdata

# Create non-root user
RUN addgroup -g 1000 icalmerge && \
    adduser -D -u 1000 -G icalmerge icalmerge

# Create directory for config
RUN mkdir -p /app/config && \
    chown -R icalmerge:icalmerge /app

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/ical-merge /usr/local/bin/ical-merge

# Switch to non-root user
USER icalmerge

# Expose default port
EXPOSE 8080

# Set default environment variables
ENV ICAL_MERGE_CONFIG=/app/config/config.json

# Run the binary
ENTRYPOINT ["/usr/local/bin/ical-merge"]
