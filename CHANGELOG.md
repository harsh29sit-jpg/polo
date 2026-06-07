# Changelog

All notable changes to this project will be documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
Versions follow [Semantic Versioning](https://semver.org/).

---

## [Unreleased]

Nothing pending.

---

## [0.2.0] — 2026-06-07

### Added

**Store layer**
- `gc()` — purge retracted facts older than an optional cutoff; supports dry-run and per-branch scope
- `stats()` — aggregate `StoreStats` (fact count, retracted count, transaction count, namespace/branch counts, oldest/newest HLC, estimated storage bytes)
- `list_entities()` — enumerate distinct entity IDs with live facts on a branch
- `list_attrs()` — enumerate distinct attrs for a specific entity on a branch
- `put_tag / get_tag / delete_tag / list_tags` — global label → TxId mapping; tags survive namespace and branch boundaries
- `tags` table added to SQLite schema (migration is additive; existing databases are unaffected)
- `bulk_record()` on `Db` — atomically record multiple facts under one transaction ID and timestamp
- `apply_retention()` on `Db` — apply a `RetentionPolicy` across a namespace (max age, max versions, only-retracted filtering, dry-run)

**API**
- `GET /v1/:ns/entities?branch=` — list entity IDs
- `GET /v1/:ns/dump` — NDJSON export of all facts on all branches in a namespace
- `POST /v1/:ns/restore` — idempotent NDJSON import (uses original fact ID as idempotency key)
- `GET /v1/tags` — list all tags
- `GET /v1/tags/:label` — resolve a tag to a transaction ID
- `PUT /v1/tags/:label` — create or overwrite a tag
- `DELETE /v1/tags/:label` — remove a tag
- `GET /v1/stats` — store-wide statistics

**Client**
- `PoloClient::put_tag / get_tag / delete_tag / list_tags`
- `PoloClient::stats`
- `PoloClient::dump / restore`
- `PoloClient::list_entities`

**CLI**
- `polo tag list / put / get / del` — tag management
- `polo stats` — print store statistics
- `polo entities` — list entities on a branch
- `polo dump` — stream NDJSON to stdout
- `polo restore [--file <path>]` — import NDJSON from file or stdin

**Error types**
- `Error::TagNotFound(String)` — returned by `get_tag` and `delete_tag` when the label is absent; also matched by `is_not_found()`

**Tests**
- `internal/polo-store/tests/conformance.rs` — 28 test cases, every assertion executed against both `MemoryStore` and `SqliteStore` to guarantee behavioral parity
- `pkg/polo-core/tests/core_tests.rs` — unit tests for HLC clock (monotonicity, concurrent safety, observe-advance, display roundtrip), typed value roundtrips, schema validation (strict/non-strict, type mismatch, `Any`), GC/retention struct defaults, PQL parse and eval (filter, LIKE, LIMIT), scan query defaults, and all `Error::is_not_found` variants
- `internal/polo-server/tests/api_tests.rs` — HTTP integration tests using `tower::ServiceExt::oneshot` against an in-memory store: health, version, namespace CRUD, fact record/get/retract, history, snapshot, branch lifecycle, diff, tag lifecycle, stats, PQL query, bearer token auth

---

## [0.1.0] — 2026-05-01

### Added

- Bi-temporal causal fact ledger with valid-time and transaction-time axes
- Hybrid Logical Clock (HLC, Kulkarni & Demirbas 2014) — u64 encoding (48-bit physical ms + 16-bit logical counter), lock-free CAS loop, rejects clocks >60 s ahead
- Namespace-scoped storage — each namespace has its own branch tree, merge policy (`LastWriteWins` / `FirstWriteWins` / `ErrorOnConflict`), and optional schema
- Causal chain tracking — every `Fact` and `Transaction` carries an optional `caused_by: Option<TxId>`
- Typed values — `Value` enum: `Str / Int / Float / Bool / Json / Null`
- Git-style branching — fork, merge, diff; configurable conflict resolution per namespace
- Three-way merge with per-namespace conflict policy
- Idempotency cache — duplicate detection via caller-supplied idempotency key (SQLite table)
- Schema validation — per-attr type enforcement with strict mode (rejects unknown attrs)
- Polo Query Language (PQL) — `SELECT … FROM … WHERE … EFFECTIVE AT … AS OF … ORDER BY … LIMIT`
- WebSocket streaming — bi-directional event bus; clients can filter by namespace/branch/entity/attr
- Bearer token authentication middleware
- CORS middleware (configurable origin or wildcard)
- SQLite storage backend — WAL mode, `PRAGMA synchronous=NORMAL`, bundled libsqlite3
- In-memory store — `Arc<RwLock<State>>`, suitable for tests and ephemeral use
- `polod` server binary — CLI flags + environment variable configuration, structured tracing
- `polo` CLI — `record`, `asof`, `effective`, `history`, `snapshot`, `retract`, `branch` (list/create/delete), `diff`, `merge`, `query`, `ping`, `version`
- Docker support — multi-stage `Dockerfile`, `docker-compose.yml` for local development
- GitHub Actions CI workflow

[Unreleased]: https://github.com/harsh29sit-jpg/polo/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/harsh29sit-jpg/polo/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/harsh29sit-jpg/polo/releases/tag/v0.1.0
