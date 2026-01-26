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

### Hot-Reload Configuration

Simply edit `config.json` and save - changes are automatically detected and applied within ~2 seconds. No server restart needed!

The config watcher:
- Uses polling (not filesystem events) for Docker bind mount compatibility
- Validates new config before applying
- Keeps old config if new one is invalid
- Logs all reload attempts

## Environment Variables

Configuration can be overridden with environment variables prefixed with `ICAL_MERGE_`:

```bash
export ICAL_MERGE_SERVER__PORT=9090
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

## License

MIT
