//! `yourmemory token` — manage access tokens for the network dashboard (`serve`).
//!
//! Tokens gate HTTP access; the local stdio MCP/CLI path is unaffected. A token
//! carries per-wing grants (`read`/`write`/`admin`). Use `*` as the wing for a
//! global grant (`*:admin` = full token management).

use anyhow::{bail, Context, Result};
use yourmemory_core::access::{Grant, GrantLevel, GLOBAL_WING};
use yourmemory_core::storage::Palace;

use clap::Subcommand;

#[derive(Subcommand)]
pub enum TokenCommand {
    /// Create a token. The secret is printed once and never recoverable.
    Create {
        /// Human-readable label, e.g. "ci" or "alice-laptop".
        #[arg(long)]
        label: String,
        /// Grant in `wing:level` form (repeatable). level ∈ read|write|admin. Wing `*` is global.
        #[arg(long = "grant", value_name = "WING:LEVEL")]
        grants: Vec<String>,
    },
    /// List tokens (secrets are never shown).
    List,
    /// Soft-revoke a token by id (preserved in the audit trail).
    Revoke {
        id: i64,
    },
    /// Add or update a single grant on an existing token.
    Grant {
        id: i64,
        /// `wing:level` — level ∈ read|write|admin. Wing `*` is global.
        #[arg(value_name = "WING:LEVEL")]
        grant: String,
    },
}

fn open_palace() -> Result<Palace> {
    let cwd = std::env::current_dir().context("cannot determine current directory")?;
    Palace::open(&cwd).context("Palace not initialised — run `yourmemory init` first")
}

/// Parse a `wing:level` grant spec. The wing may itself be `*`.
fn parse_grant(spec: &str) -> Result<Grant> {
    let (wing, level) = spec
        .rsplit_once(':')
        .with_context(|| format!("grant '{spec}' must be in WING:LEVEL form"))?;
    let wing = wing.trim();
    if wing.is_empty() {
        bail!("grant '{spec}' has an empty wing");
    }
    let level = GrantLevel::parse(level)
        .with_context(|| format!("grant '{spec}': level must be read, write, or admin"))?;
    Ok(Grant { wing: wing.to_string(), level })
}

pub fn run(cmd: &TokenCommand) -> Result<()> {
    let palace = open_palace()?;
    match cmd {
        TokenCommand::Create { label, grants } => {
            if grants.is_empty() {
                bail!("a token needs at least one --grant (e.g. --grant engineering:read)");
            }
            let parsed: Vec<Grant> = grants.iter().map(|g| parse_grant(g)).collect::<Result<_>>()?;
            let (token, secret) = palace
                .storage
                .create_access_token(label, &parsed)
                .context("cannot create token")?;
            println!("Created token #{} ({})", token.id, token.label);
            for g in &parsed {
                println!("  grant: {}:{}", g.wing, g.level.as_str());
            }
            println!();
            println!("Secret (shown once — store it now):");
            println!("  {secret}");
        }
        TokenCommand::List => {
            let tokens = palace.storage.list_access_tokens().context("cannot list tokens")?;
            if tokens.is_empty() {
                println!("No tokens. Create one with `yourmemory token create`.");
                return Ok(());
            }
            for t in &tokens {
                let grants = palace.storage.list_grants(t.id)?;
                let grant_str = grants
                    .iter()
                    .map(|g| format!("{}:{}", g.wing, g.level.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ");
                let status = if t.is_revoked() { "REVOKED" } else { "active" };
                println!(
                    "#{:<3} {:<20} [{}] grants: {}",
                    t.id,
                    t.label,
                    status,
                    if grant_str.is_empty() { "(none)" } else { &grant_str }
                );
            }
        }
        TokenCommand::Revoke { id } => {
            if palace.storage.revoke_access_token(*id).context("cannot revoke token")? {
                println!("Revoked token #{id}.");
            } else {
                println!("No active token #{id} to revoke.");
            }
        }
        TokenCommand::Grant { id, grant } => {
            if palace.storage.get_access_token(*id)?.is_none() {
                bail!("no token #{id}");
            }
            let g = parse_grant(grant)?;
            palace.storage.set_grant(*id, &g.wing, g.level).context("cannot set grant")?;
            let scope = if g.wing == GLOBAL_WING { " (global)" } else { "" };
            println!("Granted {}:{}{} on token #{id}.", g.wing, g.level.as_str(), scope);
        }
    }
    Ok(())
}
