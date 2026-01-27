# Claude Code Project Guide: iCal Merge

## Project Overview

A lightweight async Rust HTTP server that fetches iCal calendars from URLs, applies per-source filters and modifiers, merges them, and serves the result via HTTP endpoints.

**Key Use Case**: Combine multiple work/personal calendars with filtering (e.g., "only meetings, no optional events") and modifications (e.g., prefix summaries with tags).

## Architecture

```
HTTP Request → Server Handler → Merge Logic → [Concurrent Fetching] → Filter → Modify → Serialize
                                                     ↓
                                            Source 1, Source 2, ...
```

### Core Flow
1. **Fetch**: Concurrent async HTTP fetches of source calendars
2. **Parse**: iCal text → structured Calendar/Event types
3. **Filter**: Apply allow/deny rules per source
4. **Modify**: Regex replacements on event summaries per source
5. **Merge**: Combine filtered events from all sources
6. **Serialize**: Back to valid iCal format
7. **Serve**: HTTP response with `text/calendar` content type

## Module Organization

- **config.rs**: Figment-based config (JSON + env vars), validation
- **error.rs**: Application error type with thiserror
- **ical/**: Calendar/Event wrappers around `icalendar` crate
  - `types.rs`: Wrapper types with convenient accessors
  - `parser.rs`: Parse/serialize functions
- **filter/**: Filtering and modification logic
  - `rules.rs`: `CompiledFilter` with allow/deny regex rules
  - `modifier.rs`: `CompiledModifier` for summary replacements
- **fetcher.rs**: HTTP client wrapper with timeout, User-Agent
- **merge.rs**: Orchestrates fetch → filter → modify → merge
- **server.rs**: Axum routes, handlers, AppState
- **main.rs**: CLI, server startup

## Key Design Decisions

### Config Hot-Reloading
**Location**: `watcher.rs`, `server.rs:AppState`

The config file is automatically watched for changes using `notify::PollWatcher`:
- **PollWatcher** is used (not event-based) for Docker bind mount compatibility
- Poll interval: 2 seconds in production, 500ms in tests
- Config is behind `Arc<RwLock<Config>>` for thread-safe updates
- Invalid configs are rejected - old config stays active
- No server restart needed for config changes

When config changes:
1. File watcher detects modification
2. New config is loaded and validated
3. If valid: atomically swapped in via RwLock
4. If invalid: logged as error, old config retained
5. In-flight requests use consistent config snapshot (read lock)

### Filter Logic (IMPORTANT!)
**Location**: `filter/rules.rs:should_include()`

```rust
match (has_allow, has_deny) {
    (false, false) => true,           // No rules = allow all
    (true, false) => matches_allow,   // Only allow = must match
    (false, true) => !matches_deny,   // Only deny = must not match
    (true, true) => !matches_deny && matches_allow, // Both = AND logic
}
```

This is the core business logic - do not change without careful consideration.

### Partial Failure Handling
**Location**: `merge.rs:merge_calendars()`

Sources that fail to fetch/parse are logged as errors but don't fail the entire request. This is intentional - we serve whatever data we can get. The `MergeResult` type captures both events and errors.

### Per-Source Configuration
Each source has its own filters and modifiers. This is not global - a filter on source A doesn't affect source B. This allows combining filtered and unfiltered sources.

### Concurrency
Sources are fetched concurrently using `futures::future::join_all`. This is critical for performance when merging many sources.

## Dependencies Rationale

- **tokio**: Async runtime (required for reqwest and axum)
- **axum**: Modern, ergonomic HTTP framework
- **reqwest**: Async HTTP client with rustls (no OpenSSL dependency)
- **figment**: Flexible config (JSON file + env vars)
- **icalendar**: Battle-tested RFC 5545 parsing/serialization
- **regex**: Filter patterns and modifiers
- **futures**: For `join_all` (concurrent fetching)
- **notify**: File watching for config hot-reload (PollWatcher for Docker compatibility)

## Code Conventions

### Error Handling
- Use `?` operator extensively - errors bubble up naturally
- `Result<T>` is `std::result::Result<T, crate::error::Error>`
- Partial failures in merge return `MergeResult` with both events and errors

### Async/Await
- All I/O operations are async
- Use `tokio::test` macro for async tests
- Mock servers (wiremock) for testing HTTP fetches

### Testing Strategy
1. **Unit tests**: In each module, test individual functions
2. **Integration tests**: `tests/integration.rs` - full flow with mock servers
3. **Fixtures**: `tests/fixtures/*.ics` - realistic iCal files

### Wrapper Pattern
`icalendar::Event` is wrapped in `ical::Event` to:
- Provide convenient accessors (`summary()`, `description()`, `uid()`)
- Allow mutation without complex borrow issues
- Abstract away property traversal complexity

### Thread Safety with RwLock
Config is accessed via `Arc<RwLock<Config>>`:
- Handlers acquire **read lock**, clone needed calendar config, then release
- Watcher acquires **write lock** only during reload
- This pattern minimizes lock contention - handlers don't block each other
- Calendar configs must be `Clone` to enable lock-free processing

## Common Tasks

### Testing Config Reload
Use the helper function for fast testing:
```rust
start_config_watcher_with_interval(state, Duration::from_millis(500))
```
Production uses 2 second interval, but tests need faster polling.

### Adding a New Filter Field
1. Update `Event` accessors in `ical/types.rs` if needed
2. Add field name to match in `filter/rules.rs:CompiledFilterRule::matches()`
3. Update `default_filter_fields()` in `config.rs` if it should be default

### Adding a New Route
1. Add handler function in `server.rs`
2. Add route in `create_router()`
3. Add test in `server.rs` tests module
4. Use `Path`, `State`, etc. extractors as needed

### Modifying iCal Properties Beyond Summary
Currently only summary is modified. To extend:
1. Add setter methods to `ical::Event`
2. Update `CompiledModifier` to accept field parameter
3. Update config schema in `config.rs`

### Running Tests
```bash
mise run test       # Uses mise task
cargo test          # Direct cargo
cargo test --lib    # Only library tests
cargo test --test integration  # Only integration tests
```

## Important Implementation Details

### Event UID Extraction
`ical::Event::uid()` manually traverses properties because icalendar crate doesn't expose a getter. The property is a tuple `(String, Property)`.

### Calendar Clone Issue
`icalendar::Calendar` doesn't implement `Clone`, so our wrapper doesn't derive it. Be careful when needing to clone - extract events instead.

### Regex Compilation
Filters and modifiers compile regexes once during config load. Don't compile regexes per-event - it's expensive.

### Borrow Checker in Modifiers
`modifier.apply()` must clone the summary string before modifying because we need both immutable (read) and mutable (write) access to the event.

### PollWatcher for Docker Compatibility
`notify::PollWatcher` is used instead of event-based watchers because:
- Filesystem events often don't work through Docker bind mounts
- Polling is reliable across all platforms and mount types
- 2 second interval is acceptable latency for config changes
- The watcher is kept alive by moving it into the tokio task

## Configuration Schema

```json
{
  "server": {
    "bind_address": "127.0.0.1",  // defaults provided
    "port": 8080
  },
  "calendars": {
    "calendar-id": {              // used in URL path
      "sources": [{
        "url": "https://...",      // required
        "filters": {               // optional
          "allow": [{"pattern": "...", "fields": ["summary"]}],
          "deny": [{"pattern": "..."}]
        },
        "modifiers": [{            // optional
          "pattern": "...",        // regex to find
          "replacement": "..."     // replacement (supports $1, etc.)
        }]
      }]
    }
  }
}
```

Environment variable overrides:
- Config file values: `ICAL_MERGE_SERVER__PORT=9090`
- CLI arguments: `ICAL_MERGE_CONFIG=/path/to/config.json`, `ICAL_MERGE_BIND=0.0.0.0`, `ICAL_MERGE_PORT=9090`

## Testing Best Practices

### Mock Server Pattern
```rust
let mock_server = MockServer::start().await;
Mock::given(method("GET"))
    .and(path("/cal.ics"))
    .respond_with(ResponseTemplate::new(200).set_body_string(ICAL_DATA))
    .mount(&mock_server)
    .await;
```

### Axum Testing
Use `tower::ServiceExt::oneshot()` for one-off requests:
```rust
let app = create_router(state);
let response = app.oneshot(request).await.unwrap();
```

### Event Creation in Tests
Helper pattern:
```rust
fn create_event(summary: &str, description: Option<&str>) -> Event {
    let mut event = icalendar::Event::new();
    event.summary(summary);
    if let Some(desc) = description {
        event.description(desc);
    }
    Event::new(event)
}
```

## Known Limitations & Future Work

### Current Limitations
1. No caching - every request fetches sources fresh
2. No authentication for source URLs or served endpoints
3. Only summary field is modifiable
4. No health check or metrics endpoints
5. Config reload has ~2 second latency (poll interval)
6. Vendor X-* properties may be lost in round-trip

### Potential Enhancements
- TTL-based caching (add `cached` crate)
- Basic Auth or Bearer token support
- Modify description, location, etc.
- Prometheus metrics endpoint
- WebDAV support for source calendars

## Troubleshooting

### "Calendar not found" 404
- Check calendar ID in URL matches config keys
- Calendar IDs are case-sensitive

### Events disappearing
- Check filter logic - likely being filtered out
- Use logs: `RUST_LOG=ical_merge=debug cargo run`
- Verify regex patterns match expected fields

### Timeout errors
- Default is 30s, adjust `Fetcher::with_timeout()` if needed
- Check source URL is reachable: `curl -I <url>`

### Parse errors
- Verify source returns valid iCal (not HTML error page)
- Check `Content-Type` of source
- Test parsing independently: add `--nocapture` to see parse details

## Development Workflow

1. Make changes
2. Run tests: `mise run test`
3. Commit: `jj commit -m "Description"`
4. Build: `cargo build --release`
5. Test manually: `./target/release/ical-merge -c test-config.json`

## Docker Deployment

### Multi-Stage Dockerfile

The Dockerfile uses two stages:
1. **Builder** (rust:alpine): Compiles the Rust binary with musl for static linking
2. **Runtime** (alpine:latest): Minimal image with just the binary and CA certs

**Why alpine for runtime?**
- Small size (~15MB total)
- Has shell for debugging
- CA certificates package available
- Timezone data for proper time handling

**Alternatives considered:**
- `scratch`: Even smaller but no shell, harder to debug, no CA certs
- `distroless`: Good middle ground but alpine is more familiar

### Building

```bash
docker build -t ical-merge .
```

Build time optimizations:
- Uses `--locked` to ensure reproducible builds
- `.dockerignore` excludes target/ directory
- Multi-stage keeps runtime image small

### Running

Key considerations:
- Bind mount config.json for hot-reload to work
- Mount as read-only (`:ro`) for security
- Set `RUST_LOG` for logging level
- Expose appropriate port
- Non-root user (icalmerge:1000) for security

### Config Hot-Reload in Docker

The PollWatcher was specifically chosen because it works reliably with Docker bind mounts:
- Filesystem events often don't propagate through bind mounts
- Polling always works, regardless of host OS or mount type
- 2 second interval is acceptable for config changes

## Quick Reference

**Start server**: `cargo run -- -c config.json`
**Run tests**: `mise run test`
**Build release**: `cargo build --release`
**Build Docker**: `docker build -t ical-merge .`
**Run Docker**: `docker-compose up -d`
**Fetch calendar**: `curl http://localhost:8080/ical/{id}`
**Override port**: `cargo run -- --port 9090`
**Debug logging**: `RUST_LOG=debug cargo run`
