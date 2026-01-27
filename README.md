# iCal Merge

A lightweight async Rust tool that fetches, filters, modifies, and merges iCal calendars from the web, exposing them via HTTP endpoints.

## Configuration

Configuration files can be in either **JSON** or **TOML** format. If no config is specified, the tool will auto-detect `config.toml` or `config.json` in the current directory.

### JSON Example

Create a `config.json` file:

```json
{
  "server": {
    "bind_address": "0.0.0.0",
    "port": 8080
  },
  "calendars": {
    "my-calendar": {
      "sources": [
        {
          "url": "https://example.com/calendar.ics",
          "steps": [
            {
              "type": "deny",
              "patterns": ["(?i)optional"],
              "fields": ["summary"]
            },
            {
              "type": "allow",
              "patterns": ["(?i)meeting"]
            },
            {
              "type": "replace",
              "pattern": "^Meeting:",
              "replacement": "[WORK] ",
              "field": "summary"
            },
            {
              "type": "case",
              "transform": "title"
            }
          ]
        }
      ],
      "steps": []
    }
  }
}
```

### TOML Example

Or create a `config.toml` file:

```toml
[server]
bind_address = "0.0.0.0"
port = 8080

[calendars.my-calendar]

[[calendars.my-calendar.sources]]
url = "https://example.com/calendar.ics"

# Deny optional events
[[calendars.my-calendar.sources.steps]]
type = "deny"
patterns = ["(?i)optional"]
fields = ["summary"]

# Only allow meetings
[[calendars.my-calendar.sources.steps]]
type = "allow"
patterns = ["(?i)meeting"]

# Add prefix
[[calendars.my-calendar.sources.steps]]
type = "replace"
pattern = "^Meeting:"
replacement = "[WORK] "
field = "summary"

# Transform to title case
[[calendars.my-calendar.sources.steps]]
type = "case"
transform = "title"

# No calendar-level steps
calendars.my-calendar.steps = []
```

### Processing Steps

Events are processed through a pipeline of steps. Steps are applied in order, and processing stops if an event is rejected by a filter.

#### Available Step Types

**Allow** - Only keep events matching patterns:

```json
{
  "type": "allow",
  "patterns": ["(?i)meeting", "(?i)standup"],
  "mode": "any",
  "fields": ["summary", "description"]
}
```

- `patterns`: Regex patterns to match (required)
- `mode`: `"any"` (default) or `"all"` - whether any or all patterns must match
- `fields`: Fields to search (defaults to `["summary", "description"]`)

**Deny** - Reject events matching patterns:

```json
{
  "type": "deny",
  "patterns": ["(?i)optional", "(?i)cancelled"],
  "mode": "any",
  "fields": ["summary"]
}
```

- Same parameters as allow

**Replace** - Modify event text with regex:

```json
{
  "type": "replace",
  "pattern": "^Meeting:",
  "replacement": "[WORK] ",
  "field": "summary"
}
```

- `pattern`: Regex pattern to find (required)
- `replacement`: Replacement text, supports capture groups like `$1` (defaults to `""`)
- `field`: Field to modify - `"summary"`, `"description"`, or `"location"` (defaults to `"summary"`)

**Case** - Transform text case:

```json
{
  "type": "case",
  "transform": "title",
  "field": "summary"
}
```

- `transform`: `"lower"`, `"upper"`, `"sentence"`, or `"title"` (required)
- `field`: Field to transform (defaults to `"summary"`)

**Strip** - Remove event components:

```json
{
  "type": "strip",
  "field": "reminder"
}
```

- `field`: `"reminder"` (only supported field currently)

### Step Execution

- **Source-level steps**: Applied to events from that specific source before merging
- **Calendar-level steps**: Applied to all events after merging from all sources
- **Order matters**: Steps are executed sequentially in the order defined

### Filter Logic

For allow/deny steps:

- When only allow steps are present, events must match at least one
- When only deny steps are present, events must not match any
- When both are present, events must not match any deny AND must match at least one allow
- `mode: "all"` requires all patterns to match; `mode: "any"` requires at least one

### Calendar References

You can reference other calendars as sources:

```json
{
  "calendars": {
    "base": {
      "sources": [{"url": "https://example.com/cal.ics"}]
    },
    "derived": {
      "sources": [
        {
          "calendar": "base",
          "steps": [
            {
              "type": "replace",
              "pattern": "^",
              "replacement": "[DERIVED] "
            }
          ]
        }
      ]
    }
  }
}
```

This allows composing calendars from other calendars with additional processing.

### Supported URL Schemes

The tool supports standard HTTP(S) URLs as well as calendar-specific schemes:

- `http://` and `https://` - Standard web URLs
- `webcal://` - Automatically converted to `http://`
- `webcals://` - Automatically converted to `https://`

Example:

```json
{
  "url": "webcal://example.com/calendar.ics"
}
```

## Usage

### Local Development

Run the server (auto-detects `config.toml` or `config.json`):

```bash
cargo run
```

Or specify a config file explicitly:

```bash
cargo run -- -c config.json
```

Run with custom bind/port:

```bash
cargo run -- serve --bind 0.0.0.0 --port 9090
```

Show events from a calendar:

```bash
cargo run -- show my-calendar
```

Output calendar as iCal format:

```bash
cargo run -- ical my-calendar > output.ics
```

Access merged calendars via HTTP:

```bash
curl http://localhost:8080/ical/my-calendar
```

### Docker

**Using docker run:**

```bash
# With config.toml
docker run -d \
  --name ical-merge \
  -p 8080:8080 \
  -v $(pwd)/config.toml:/app/config.toml:ro \
  -e RUST_LOG=ical_merge=info \
  ical-merge

# Or with config.json
docker run -d \
  --name ical-merge \
  -p 8080:8080 \
  -v $(pwd)/config.json:/app/config.json:ro \
  -e RUST_LOG=ical_merge=info \
  ical-merge
```

**Using docker-compose:**

```bash
docker-compose up -d
```

The docker-compose.yml is already configured to:

- Mount your local config file (supports both `.toml` and `.json`)
- Expose port 8080
- Auto-restart on failure
- Use PollWatcher for reliable config hot-reload with bind mounts

### Hot-Reload Configuration

Simply edit your config file (`config.toml` or `config.json`) and save - changes are automatically detected and applied within ~2 seconds. No server restart needed!

The config watcher:

- Uses polling (not filesystem events) for Docker bind mount compatibility
- Validates new config before applying
- Keeps old config if new one is invalid
- Logs all reload attempts
- Works with both TOML and JSON formats

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
# Specify config file (optional - auto-detects config.toml or config.json if not set)
export ICAL_MERGE_CONFIG=/path/to/config.toml

# Override server settings
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
  -v $(pwd)/config.toml:/app/config.toml:ro \
  -e ICAL_MERGE_CONFIG=/app/config.toml \
  -e RUST_LOG=ical_merge=info \
  ical-merge serve --bind 0.0.0.0 --port 9090
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
