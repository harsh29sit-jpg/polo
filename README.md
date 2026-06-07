# polo

A bi-temporal, causal fact ledger written in Rust.

Polo stores immutable facts with two time dimensions — when something was **true** in the world (valid time) and when you **recorded** it (transaction time, tracked via Hybrid Logical Clock). Facts are organised into namespaces and every transaction optionally carries a causal reference, so you can reconstruct the full lineage of any piece of state.

---

## Why polo?

Most databases answer "what is X right now?". Polo lets you ask:

- What was X **effective** at `2023-06-01`?
- What did we **know** about X at transaction `0001abc...`?
- Which transactions **caused** this state?
- What changed on branch `experiment` vs `main`?

It also lets you write to an isolated **branch**, compare the diff, and merge — or discard — without touching production state.

---

## Quick start

```bash
# build
make build

# start the server (persists to polo.db)
./bin/polod --db ./polo.db

# record a fact
./bin/polo record user/1 name alice

# query it back
./bin/polo asof user/1 name

# create a branch and experiment
./bin/polo branch create staging
./bin/polo -branch staging record user/1 name "bob"
./bin/polo -branch staging history user/1 name

# see what staging changed relative to main
./bin/polo diff staging

# merge when happy
./bin/polo merge staging --into main

# run a PQL query
./bin/polo query "SELECT entity, attr, value FROM default WHERE entity = 'user/1'"
```

---

## Concepts

### Facts

The atomic unit. A fact records that `entity.attr = value` was true over some valid-time interval. Once written, facts are never updated in place — a retraction writes a new fact that marks the interval closed.

```
namespace  entity    attr   value       valid_from           valid_to
default    user/1    name   alice       2024-01-01T00:00:00Z <open>
default    user/1    email  a@a.com     2024-01-01T00:00:00Z <open>
```

### Namespaces

Namespaces are logical containers within a single store. Each namespace gets its own branch tree and can have a schema and a merge policy. The default namespace is called `default`.

### Causal chains

Every transaction can carry a `caused_by` reference — the `TxId` of the transaction that triggered it. This lets you traverse the causal graph of your data: "this config was changed because that deploy happened because that ticket was closed".

### Hybrid Logical Clocks

Polo timestamps transactions with an HLC (Kulkarni & Demirbas, 2014) rather than wall-clock time. The HLC is a `u64` — 48 bits of physical milliseconds plus a 16-bit logical counter — so it's monotonically increasing even if wall time jumps backwards, and it encodes causality across distributed writes.

### Branches

Branches fork the fact tree at a point in HLC time. Writes to a branch are isolated. When you're ready, `merge` replays the branch's facts onto the target. Conflicts are surfaced and you choose the resolution strategy per-namespace (`last-write-wins`, `first-write-wins`, or `error`).

---

## HTTP API

The server (`polod`) speaks JSON over HTTP/1.1.

| Method | Path | Description |
|--------|------|-------------|
| POST | `/v1/{ns}/facts` | Record a fact |
| GET | `/v1/{ns}/facts/{id}` | Get fact by ID |
| DELETE | `/v1/{ns}/facts/{id}` | Retract a fact |
| GET | `/v1/{ns}/asof` | Point-in-time (transaction time) query |
| GET | `/v1/{ns}/effective` | Point-in-time (valid time) query |
| GET | `/v1/{ns}/history` | Full attribute history |
| GET | `/v1/{ns}/snapshot/{entity}` | Current entity state |
| GET | `/v1/{ns}/branches` | List branches |
| POST | `/v1/{ns}/branches` | Create branch |
| DELETE | `/v1/{ns}/branches/{name}` | Delete branch |
| POST | `/v1/{ns}/merge` | Merge branch |
| GET | `/v1/{ns}/diff` | Diff two branches |
| POST | `/v1/{ns}/query` | Run a PQL query |
| GET | `/v1/{ns}/stream` | WebSocket stream of fact events |
| GET | `/v1/namespaces` | List namespaces |
| POST | `/v1/namespaces` | Create namespace |
| GET | `/healthz` | Health check |
| GET | `/version` | Build info |

### Authentication

Set `--token` on the server. Clients send `Authorization: Bearer <token>`.

---

## PQL — Polo Query Language

```sql
-- basic select
SELECT entity, attr, value, valid_from
FROM default
WHERE entity = 'user/1'
ORDER BY valid_from DESC
LIMIT 20

-- effective at a point in valid time
SELECT entity, attr, value
FROM orders
EFFECTIVE AT '2024-03-15T00:00:00Z'
WHERE attr = 'status'

-- as of a point in transaction time
SELECT entity, attr, value
FROM inventory
AS OF '0001abc0000000000000'
WHERE entity LIKE 'product/%'
```

---

## CLI reference

```
polo [flags] <command>

Flags:
  --addr   string   server address (default http://localhost:5432, env: POLO_ADDR)
  --ns     string   namespace (default "default", env: POLO_NS)
  --branch string   branch (default "main")
  --as     string   author for writes
  --json          machine-readable JSON output

Commands:
  record  <entity> <attr> <value> [--from RFC3339] [--to RFC3339] [--caused-by TxId]
  asof    <entity> <attr> [--at HLC|RFC3339]
  effective <entity> <attr> [--at RFC3339]
  history <entity> <attr>
  snapshot <entity>
  retract <fact-id>
  branch  list | create <name> [--from <branch>] | delete <name>
  diff    <branch> [--against <branch>]
  merge   <branch> --into <branch>
  query   "<PQL>"
  ping
  version
```

---

## Configuration

`polod` reads flags and the following environment variables:

| Env | Flag | Default | Description |
|-----|------|---------|-------------|
| `POLO_DB` | `--db` | `polo.db` | Path to SQLite file |
| `POLO_ADDR` | `--addr` | `0.0.0.0:5432` | Listen address |
| `POLO_TOKEN` | `--token` | (none) | Bearer token for auth |
| `POLO_LOG` | `--log` | `info` | Log level |
| `POLO_CORS_ORIGIN` | `--cors-origin` | (none) | Allowed CORS origin |
| `POLO_MAX_BODY` | `--max-body` | `4mb` | Max request body size |

---

## Building from source

Requires Rust 1.79+.

```bash
git clone https://github.com/harsyng/polo
cd polo
make build
```

---

## Docker

```bash
make docker-build
make docker-up    # starts on :5432
```

---

## License

MIT
