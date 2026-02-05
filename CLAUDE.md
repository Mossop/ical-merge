# Claude Code Project Guide: iCal Merge

> **Note**: For user-facing documentation including configuration examples, usage instructions, Docker deployment, and environment variables, refer to [README.md](README.md). This guide focuses on architecture, implementation details, and development practices.

## Project Overview

A lightweight async Rust HTTP server that fetches iCal calendars from URLs or references other virtual calendars, applies configurable processing steps (filter, modify, transform), merges them, and serves the result via HTTP endpoints.

**Key Use Case**: Combine multiple work/personal calendars with processing steps (e.g., "only meetings, no optional events", "add prefixes", "transform to title case") to create customized merged calendars.

## Architecture

```
HTTP Request → Server Handler → Merge Logic → [Concurrent Fetching/Resolution] → Process Steps → Serialize
                                                     ↓
                                    URL Sources + Calendar References
```

### Core Flow
1. **Resolve**: Fetch HTTP sources concurrently OR resolve calendar references recursively
2. **Parse**: iCal text → structured Calendar/Event types
3. **Process Steps (per source)**: Execute pipeline of steps (allow, deny, replace, case, strip)
4. **Merge**: Combine processed events from all sources
5. **Process Steps (calendar-level)**: Execute additional steps on merged events
6. **Deduplicate**: Remove duplicate events by UID
7. **Serialize**: Back to valid iCal format
8. **Serve**: HTTP response with `text/calendar` content type

## Module Organization

- **config.rs**: Figment-based config (JSON/TOML + env vars), validation, cycle detection
- **error.rs**: Application error type with thiserror
- **ical/**: Calendar/Event wrappers around `icalendar` crate
  - `types.rs`: Wrapper types with convenient accessors and mutators
  - `parser.rs`: Parse/serialize functions
- **filter/**: Processing pipeline logic
  - `steps.rs`: `CompiledStep` enum (Allow, Deny, Replace, Case, Strip) with apply logic
- **fetcher.rs**: HTTP client wrapper with timeout, User-Agent, webcal:// support
- **merge.rs**: Orchestrates fetch/resolve → process steps → merge → deduplicate
- **server.rs**: Axum routes, handlers, AppState
- **watcher.rs**: Config file hot-reload with PollWatcher
- **main.rs**: CLI, config auto-detection, server startup

## Key Design Decisions

### Config Hot-Reloading
**Location**: `watcher.rs`, `server.rs:AppState`

The config file (JSON or TOML) is automatically watched for changes using `notify::PollWatcher`:
- **PollWatcher** is used (not event-based) for Docker bind mount compatibility
- Poll interval: 2 seconds in production, 500ms in tests
- Config is behind `Arc<RwLock<Config>>` for thread-safe updates
- Invalid configs are rejected - old config stays active
- No server restart needed for config changes
- Works with both JSON and TOML formats

When config changes:
1. File watcher detects modification
2. New config is loaded and validated (format detected by extension)
3. If valid: atomically swapped in via RwLock
4. If invalid: logged as error, old config retained
5. In-flight requests use consistent config snapshot (read lock)

### Processing Steps Pipeline (IMPORTANT!)
**Location**: `filter/steps.rs`

Events are processed through a sequential pipeline of steps. Each step type:

- **Allow**: Filters events to only those matching patterns (stops pipeline if no match)
- **Deny**: Rejects events matching patterns (stops pipeline if match)
- **Replace**: Applies regex replacement to specified field (summary/description/location)
- **Case**: Transforms text case (lower/upper/sentence/title) on specified field
- **Strip**: Removes components (currently: reminders)

Filter logic (Allow/Deny steps):
```rust
match (has_allow, has_deny) {
    (false, false) => true,           // No rules = allow all
    (true, false) => matches_allow,   // Only allow = must match
    (false, true) => !matches_deny,   // Only deny = must not match
    (true, true) => !matches_deny && matches_allow, // Both = AND logic
}
```

This is the core business logic - do not change without careful consideration.

**Step execution order matters**: Steps are applied sequentially. If a filter step (allow/deny) rejects an event, remaining steps are skipped for that event.

### Case Transformations
**Location**: `config.rs:CaseTransform`, `filter/steps.rs:Case`

Four case transformation modes available:
- **lower**: Converts to lowercase (`MEETING` → `meeting`)
- **upper**: Converts to uppercase (`meeting` → `MEETING`)
- **sentence**: First character uppercase, rest lowercase (`URGENT MEETING` → `Urgent meeting`)
- **title**: Each word capitalized (`weekly team standup` → `Weekly Team Standup`)

Title case implementation handles all-uppercase input correctly by explicitly lowercasing remaining characters after capitalizing the first character of each word.

### Partial Failure Handling
**Location**: `merge.rs:merge_calendars()`

Sources that fail to fetch/parse are logged as errors but don't fail the entire request. This is intentional - we serve whatever data we can get. The `MergeResult` type captures both events and errors.

### Per-Source Configuration
Each source has its own processing steps pipeline. Steps are not global - steps on source A don't affect source B. This allows combining filtered and unfiltered sources with different transformations.

### Concurrency
Sources are fetched concurrently using `futures::future::join_all`. This is critical for performance when merging many sources.

### Calendar References
**Location**: `config.rs:SourceConfig`, `merge.rs:merge_calendars()`

Sources can reference other virtual calendars instead of URLs:

```rust
pub enum SourceConfig {
    Url { url: String, steps: Vec<Step> },
    Calendar { calendar: String, steps: Vec<Step> },
}
```

Key behaviors:
- **Recursive resolution**: `merge_calendars` is called recursively for calendar references
- **Source-level steps apply**: Steps defined on the reference are applied to the referenced calendar's events
- **Cycle detection**: Config validation detects cycles (A→B→A) and self-references using DFS
- **No HTTP requests**: Calendar resolution is internal, no self-HTTP requests
- **Same error handling**: Errors from referenced calendars propagate the same way as URL fetch errors

This enables composing derived calendars (e.g., "work" calendar with prefix added, combined with "personal" calendar).

### Calendar Auto-Detection
**Location**: `main.rs:find_config_file()`

When no config file is specified via `-c` or `ICAL_MERGE_CONFIG`, the tool searches for:
1. `config.toml` (checked first)
2. `config.json` (fallback)
3. Error if neither found

This allows running `cargo run` without arguments if a default config exists.

### Calendar URL Schemes
**Location**: `fetcher.rs:normalize_calendar_url()`

The fetcher supports both standard HTTP schemes and calendar-specific schemes:
- `http://` and `https://` - used directly
- `webcal://` - normalized to `http://` before fetching
- `webcals://` - normalized to `https://` before fetching

The `webcal://` and `webcals://` schemes are commonly used in calendar applications to indicate calendar subscription URLs. The normalization happens transparently in the fetcher, so calendar configs can use any of these schemes interchangeably.

## Dependencies Rationale

- **tokio**: Async runtime (required for reqwest and axum)
- **axum**: Modern, ergonomic HTTP framework
- **reqwest**: Async HTTP client with rustls (no OpenSSL dependency)
- **figment**: Flexible config (JSON/TOML files + env vars, format auto-detection)
- **icalendar**: Battle-tested RFC 5545 parsing/serialization
- **regex**: Pattern matching in processing steps
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

### Adding a New Step Type
1. Add new variant to `Step` enum in `config.rs`
2. Add corresponding `CompiledStep` variant in `filter/steps.rs`
3. Implement logic in `CompiledStep::apply()`
4. Add validation logic in `config.rs:validate()` if needed
5. Add tests in `filter/steps.rs`
6. Update README.md with new step documentation

### Adding a New Field to Existing Steps
1. Update `Event` accessors/mutators in `ical/types.rs` if needed
2. Add field name to match in `filter/steps.rs:CompiledStep::apply()`
3. Update `default_filter_fields()` in `config.rs` if relevant for filters
4. Update step documentation in README.md

### Adding a New Route
1. Add handler function in `server.rs`
2. Add route in `create_router()`
3. Add test in `server.rs` tests module
4. Use `Path`, `State`, etc. extractors as needed

## Important Implementation Details

### Event UID Extraction
`ical::Event::uid()` manually traverses properties because icalendar crate doesn't expose a getter. The property is a tuple `(String, Property)`.

### Calendar Clone Issue
`icalendar::Calendar` doesn't implement `Clone`, so our wrapper doesn't derive it. Be careful when needing to clone - extract events instead.

### Regex Compilation
Processing steps compile regexes once during config load (in `compile()` methods). Don't compile regexes per-event - it's expensive. The `CompiledStep` enum holds pre-compiled `Regex` instances.

### Borrow Checker in Steps
`CompiledStep::apply()` must clone field strings before modifying because we need both immutable (read) and mutable (write) access to the event. This is unavoidable with the icalendar crate's API.

### SourceConfig Enum Pattern
`SourceConfig` uses `#[serde(untagged)]` to allow both URL and Calendar sources in the same array:
```rust
#[serde(untagged)]
pub enum SourceConfig {
    Url { url: String, steps: Vec<Step> },
    Calendar { calendar: String, steps: Vec<Step> },
}
```
Serde tries each variant in order. If `calendar` field exists, it's a Calendar variant; if `url` exists, it's a Url variant. This provides clean config syntax without explicit type discriminators.

### Cycle Detection Algorithm
Calendar reference validation uses DFS with two sets:
- **visited**: All calendars seen (prevents redundant checks)
- **stack**: Calendars in current path (detects cycles)

If we encounter a calendar already in the stack, we've found a cycle. This is O(V+E) where V = calendars, E = references.

### PollWatcher for Docker Compatibility
`notify::PollWatcher` is used instead of event-based watchers because:
- Filesystem events often don't work through Docker bind mounts
- Polling is reliable across all platforms and mount types
- 2 second interval is acceptable latency for config changes
- The watcher is kept alive by moving it into the tokio task

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
3. Config reload has ~2 second latency (poll interval)
4. No health check or metrics endpoints
5. Only reminders can be stripped (no attendees, attachments, etc.)
6. Vendor X-* properties may be lost in round-trip

### Potential Enhancements
- TTL-based caching (add `cached` crate)
- Basic Auth or Bearer token support
- Prometheus metrics endpoint
- WebDAV support for source calendars
- Strip other components (attendees, attachments, alarms)
- More case transformations (kebab-case, snake_case, etc.)

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
