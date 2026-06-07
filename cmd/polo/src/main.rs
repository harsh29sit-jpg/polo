use anyhow::{bail, Context, Result};
use chrono::DateTime;
use clap::{Parser, Subcommand};

use polo_client::{PoloClient, RecordOpts};
use polo_core::{
    fact::{BranchName, TxId, Value},
    Hlc,
};

#[derive(Debug, Parser)]
#[command(name = "polo", about = "polo fact ledger CLI", version)]
struct Cli {
    /// Server address
    #[arg(long, env = "POLO_ADDR", global = true, default_value = "http://localhost:5432")]
    addr: String,

    /// Namespace
    #[arg(long, env = "POLO_NS", global = true, default_value = "default")]
    ns: String,

    /// Branch
    #[arg(long, global = true, default_value = "main")]
    branch: String,

    /// Author for writes
    #[arg(long, global = true, alias = "as")]
    author: Option<String>,

    /// Bearer token
    #[arg(long, env = "POLO_TOKEN", global = true)]
    token: Option<String>,

    /// Output raw JSON
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Record a fact
    Record {
        entity: String,
        attr: String,
        value: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        caused_by: Option<String>,
        #[arg(long)]
        idem: Option<String>,
    },

    /// Get the most recent fact at a transaction-time point
    Asof {
        entity: String,
        attr: String,
        #[arg(long)]
        at: Option<String>,
    },

    /// Get the fact valid at a point in real (valid) time
    Effective {
        entity: String,
        attr: String,
        #[arg(long)]
        at: Option<String>,
    },

    /// Full history of an attribute
    History { entity: String, attr: String },

    /// Current state of an entity (all attrs)
    Snapshot { entity: String },

    /// Retract a fact by ID
    Retract { fact_id: String },

    /// Branch management
    Branch {
        #[command(subcommand)]
        sub: BranchCmd,
    },

    /// Show what changed between two branches
    Diff {
        source: String,
        #[arg(long, default_value = "main")]
        against: String,
    },

    /// Merge a branch into another
    Merge {
        source: String,
        #[arg(long, default_value = "main")]
        into: String,
    },

    /// Run a PQL query
    Query { pql: String },

    /// Check server connectivity
    Ping,

    /// Print server version
    Version,
}

#[derive(Debug, Subcommand)]
enum BranchCmd {
    List,
    Create {
        name: String,
        #[arg(long)]
        from: Option<String>,
    },
    Delete { name: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let mut client = PoloClient::new(&cli.addr)
        .namespace(&cli.ns)
        .branch(&cli.branch);

    if let Some(author) = &cli.author {
        client = client.author(author);
    }
    if let Some(token) = &cli.token {
        client = client.with_token(token);
    }

    match cli.cmd {
        Cmd::Record {
            entity,
            attr,
            value,
            from,
            to,
            caused_by,
            idem,
        } => {
            let v = parse_value(&value);
            let valid_from = from
                .as_deref()
                .map(|s| s.parse::<chrono::DateTime<chrono::Utc>>())
                .transpose()
                .context("invalid --from timestamp")?;
            let valid_to = to
                .as_deref()
                .map(|s| s.parse::<chrono::DateTime<chrono::Utc>>())
                .transpose()
                .context("invalid --to timestamp")?;
            let caused_by = caused_by
                .as_deref()
                .map(|s| s.parse::<TxId>())
                .transpose()
                .context("invalid --caused-by value")?;

            let res = client
                .record(
                    &entity,
                    &attr,
                    v,
                    RecordOpts {
                        valid_from,
                        valid_to,
                        caused_by,
                        idempotency_key: idem,
                        ..Default::default()
                    },
                )
                .await?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&res)?);
            } else if res.was_duplicate {
                println!("duplicate  fact={} tx={}", res.fact_id, res.tx_id);
            } else {
                println!("recorded   fact={} tx={}", res.fact_id, res.tx_id);
            }
        }

        Cmd::Asof { entity, attr, at } => {
            let hlc = at
                .as_deref()
                .map(|s| s.parse::<Hlc>())
                .transpose()
                .context("invalid --at (expected hex HLC)")?;

            let fact = client.asof(&entity, &attr, hlc, Some(&cli.branch)).await?;
            print_opt_fact(fact, cli.json)?;
        }

        Cmd::Effective { entity, attr, at } => {
            let dt = at
                .as_deref()
                .map(|s| s.parse::<chrono::DateTime<chrono::Utc>>())
                .transpose()
                .context("invalid --at (expected RFC 3339)")?;

            let fact = client.effective(&entity, &attr, dt, Some(&cli.branch)).await?;
            print_opt_fact(fact, cli.json)?;
        }

        Cmd::History { entity, attr } => {
            let facts = client.history(&entity, &attr, Some(&cli.branch)).await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&facts)?);
            } else {
                for f in &facts {
                    println!(
                        "{}  {}  {}  {}  tx={}",
                        f.valid_from.format("%Y-%m-%dT%H:%M:%SZ"),
                        f.entity,
                        f.attr,
                        f.value,
                        f.tx_id,
                    );
                }
            }
        }

        Cmd::Snapshot { entity } => {
            let facts = client.snapshot(&entity, Some(&cli.branch)).await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&facts)?);
            } else {
                for f in &facts {
                    println!("{}  =  {}", f.attr, f.value);
                }
            }
        }

        Cmd::Retract { fact_id } => {
            use polo_core::fact::FactId;
            let id: FactId = fact_id.parse().context("invalid fact ID")?;
            let tx = client
                .retract(&id, Some(&cli.branch), cli.author.as_deref())
                .await?;
            println!("retracted  tx={}", tx);
        }

        Cmd::Branch { sub } => match sub {
            BranchCmd::List => {
                let branches = client.list_branches().await?;
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&branches)?);
                } else {
                    for b in &branches {
                        let parent = b
                            .parent
                            .as_ref()
                            .map(|p| format!(" (from {})", p))
                            .unwrap_or_default();
                        println!("{}{}", b.name, parent);
                    }
                }
            }
            BranchCmd::Create { name, from } => {
                client.create_branch(&name, from.as_deref()).await?;
                println!("created branch '{}'", name);
            }
            BranchCmd::Delete { name } => {
                client.delete_branch(&name).await?;
                println!("deleted branch '{}'", name);
            }
        },

        Cmd::Diff { source, against } => {
            let entries = client.diff(&source, Some(&against)).await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else if entries.is_empty() {
                println!("no differences");
            } else {
                for e in &entries {
                    let src_val = e.source.as_ref().map(|f| f.value.to_string()).unwrap_or_else(|| "(none)".into());
                    let tgt_val = e.target.as_ref().map(|f| f.value.to_string()).unwrap_or_else(|| "(none)".into());
                    println!("{}.{}  {}  →  {}", e.entity, e.attr, tgt_val, src_val);
                }
            }
        }

        Cmd::Merge { source, into } => {
            let result = client
                .merge(&source, &into, cli.author.as_deref(), None)
                .await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!(
                    "merged  facts_applied={}  conflicts={}  tx={}",
                    result.facts_applied,
                    result.conflicts.len(),
                    result.tx_id,
                );
            }
        }

        Cmd::Query { pql } => {
            let rows = client.query(&pql, Some(&cli.branch)).await?;
            if cli.json || true {
                // Always JSON for query output — tabular would need schema
                println!("{}", serde_json::to_string_pretty(&rows)?);
            }
        }

        Cmd::Ping => {
            client.ping().await?;
            println!("pong");
        }

        Cmd::Version => {
            println!("polo {}", env!("CARGO_PKG_VERSION"));
        }
    }

    Ok(())
}

fn parse_value(s: &str) -> Value {
    if s == "null" {
        return Value::Null;
    }
    if s == "true" {
        return Value::Bool(true);
    }
    if s == "false" {
        return Value::Bool(false);
    }
    if let Ok(n) = s.parse::<i64>() {
        return Value::Int(n);
    }
    if let Ok(f) = s.parse::<f64>() {
        return Value::Float(f);
    }
    if s.starts_with('{') || s.starts_with('[') {
        if let Ok(j) = serde_json::from_str::<serde_json::Value>(s) {
            return Value::Json(j);
        }
    }
    Value::Str(s.to_owned())
}

fn print_opt_fact(
    fact: Option<polo_core::fact::Fact>,
    json: bool,
) -> Result<()> {
    match fact {
        None => {
            if json {
                println!("null");
            } else {
                println!("(not found)");
            }
        }
        Some(f) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&f)?);
            } else {
                println!(
                    "{}  {}  =  {}  (tx={} valid_from={})",
                    f.entity,
                    f.attr,
                    f.value,
                    f.tx_id,
                    f.valid_from.format("%Y-%m-%dT%H:%M:%SZ"),
                );
            }
        }
    }
    Ok(())
}
