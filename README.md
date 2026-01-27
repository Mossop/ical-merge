# iCal Merge

A lightweight async Rust tool that fetches, filters, modifies, and merges iCal calendars from the web, exposing them via HTTP endpoints.

## Configuration

Create a `config.json` file:

```json
{
  "server": {
    "bind_address": "127.0.0.1",
    "port": 8080
  },
  "calendars": {
    "my-calendar": {
      "sources": [
        {
          "url": "https://example.com/calendar.ics",
          "filters": {
            "allow": [
              { "pattern": "(?i)meeting" }
            ],
            "deny": [
              { "pattern": "(?i)optional", "fields": ["summary"] }
            ]
          },
          "modifiers": [
            { "pattern": "^Meeting:", "replacement": "[WORK] " }
          ]
        }
      ]
    }
  }
}
```

### Filter Logic

- When only allow rules are present events must match at least one allow rule.
- When only deny rules are present events must not match any deny rule.
- When both are present events must not match any deny and must match at least one allow rule.
- Otherwise all events are allowed.

### Configuration Fields

- `filters.allow[].pattern`: Regex pattern to match
- `filters.allow[].fields`: Fields to search (defaults to `["summary", "description"]`)
- `filters.deny[].pattern`: Regex pattern to match
- `filters.deny[].fields`: Fields to search (defaults to `["summary", "description"]`)
- `modifiers[].pattern`: Regex pattern to find in summary
- `modifiers[].replacement`: Replacement text (supports capture groups like `$1`)

## Usage

### Local Development

Run the server:

```bash
cargo run -- --config config.json
```

Or with custom bind/port:

```bash
cargo run -- --config config.json --bind 0.0.0.0 --port 9090
```

Access merged calendars:

```bash
curl http://localhost:8080/ical/my-calendar
```

### Docker

**Using docker run:**

```bash
docker run -d \
  --name ical-merge \
  -p 8080:8080 \
  -v $(pwd)/config.json:/app/config/config.json:ro \
  -e RUST_LOG=ical_merge=info \
  ical-merge
```

**Using docker-compose:**

```bash
docker-compose up -d
```

The docker-compose.yml is already configured to:
- Mount your local config.json
- Expose port 8080
- Auto-restart on failure
- Use PollWatcher for reliable config hot-reload with bind mounts

### Hot-Reload Configuration

Simply edit `config.json` and save - changes are automatically detected and applied within ~2 seconds. No server restart needed!

The config watcher:
- Uses polling (not filesystem events) for Docker bind mount compatibility
- Validates new config before applying
- Keeps old config if new one is invalid
- Logs all reload attempts

## Environment Variables

### Server Configuration

Configuration values inside the config file can be overridden with environment variables prefixed with `ICAL_MERGE_`:

```bash
export ICAL_MERGE_SERVER__PORT=9090
cargo run
```

### CLI Arguments

CLI arguments can also be set via environment variables:

```bash
export ICAL_MERGE_CONFIG=/path/to/config.json
export ICAL_MERGE_BIND=0.0.0.0
export ICAL_MERGE_PORT=9090
cargo run
```

This is useful for Docker deployments where you want to configure everything through environment variables.

**Docker example:**

```bash
docker run -d \
  --name ical-merge \
  -p 9090:9090 \
  -v $(pwd)/config.json:/etc/ical-merge/config.json:ro \
  -e ICAL_MERGE_CONFIG=/etc/ical-merge/config.json \
  -e ICAL_MERGE_BIND=0.0.0.0 \
  -e ICAL_MERGE_PORT=9090 \
  -e RUST_LOG=ical_merge=info \
  ical-merge
```

## Testing

Run all tests:

```bash
cargo test
```

Or with mise:

```bash
mise run test
```

## Docker Image Details

The Docker image uses a multi-stage build:
- **Builder**: rust:alpine with build dependencies
- **Runtime**: alpine:latest (~10MB) with just the binary and CA certificates

Benefits:
- Minimal runtime image (~15MB total)
- Non-root user for security
- CA certificates for HTTPS requests
- Timezone data included
- Config hot-reload works with bind mounts

## License

MIT
