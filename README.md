# iCal Merge

A lightweight async Rust tool that fetches, filters, modifies, and merges iCal calendars from the web, exposing them via HTTP endpoints.

## Configuration

Configuration files can be in either **JSON** or **TOML** format. If no config is specified, the tool will auto-detect `config.toml` or `config.json` in the current directory.

The configuration defines a set of virtual calendars. Each has an ID which exposes the calendar at the `/ical/<id>` http endpoint. Each virtual calendar is composed of a set of sources which are either calendars available from a url (`http`, `https`, `webcal` and `webcals` protocols supported) or an existing virtual calendar can be used as a source.

Processing steps are applied to every event, these steps can modify and potentially reject events. Each calendar source can define a set of steps to be applied to every event from that source and then a set of global steps can be defined for the virtual calendar which will be applied to every event from every source for that calendar. The global steps apply after the steps for each source have been applied. Steps are applied sequentially and remaining steps are skipped if a step rejects an event.

### JSON Example

Create a `config.json` file:

```json
{
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

### Available Step Types

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
docker run -d \
  -p 8080:8080 \
  -v $(pwd)/config.toml:/app/config.toml:ro \
  -e RUST_LOG=ical_merge=info \
  ghcr.io/mossop/ical-merge:latest
```

**Using docker-compose:**

```bash
docker-compose up -d
```

The docker-compose.yml is already configured to:

- Mount your local config file
- Expose port 8080
- Auto-restart on failure

### Hot-Reload Configuration

Simply edit your config file (`config.toml` or `config.json`) and save - changes are automatically detected and applied within ~2 seconds. No server restart needed!

## Environment Variables

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

## Testing

Run all tests:

```bash
cargo test
```

Or with mise:

```bash
mise run test
```

### Docker Integration Tests

The project includes Docker-based integration tests that verify config hot-reload works correctly in containerized environments. These tests use testcontainers to:

- Build and start the app in a Docker container
- Mount a config file as a bind mount
- Verify initial configuration works
- Modify the config file on the host
- Verify the container detects changes and reloads configuration

To run only these tests:

```bash
cargo test --test docker_config_reload
```

**Note:** These tests require Docker to be running and take longer to execute because they build the Docker image and start containers. The tests automatically build the Docker image using the project's Dockerfile before running.

## License

MIT
