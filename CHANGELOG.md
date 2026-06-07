# Changelog

## [Unreleased]

### Added
- Initial implementation of bi-temporal causal fact ledger
- Hybrid Logical Clock (HLC) for monotonic transaction timestamps
- Namespace-scoped fact storage with pluggable merge policies
- Causal chain tracking via `caused_by` transaction references
- Git-style branching with three-way merge and conflict detection
- Polo Query Language (PQL) — SELECT/FROM/WHERE/EFFECTIVE AT/AS OF/ORDER BY/LIMIT
- WebSocket streaming for real-time fact subscription
- Bearer token authentication
- SQLite storage backend (WAL mode, bundled)
- In-memory store for tests
- Docker support
- `polod` HTTP server and `polo` CLI client
