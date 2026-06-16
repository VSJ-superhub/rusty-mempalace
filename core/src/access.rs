//! Access control for the network server (`yourmemory serve`).
//!
//! The local stdio MCP/CLI path is single-user and trusts filesystem permissions.
//! The HTTP server, by contrast, can expose the palace over a network, so it gates
//! every request behind a bearer **token** that carries per-wing **grants**.
//!
//! Pure types and scope logic live here. The SQLite-backed token store
//! (`create_access_token`, `resolve_scope`, …) is implemented on `SqliteStorage`
//! in `storage.rs`, because it needs the private connection.

use std::collections::HashMap;

use sha2::{Digest, Sha256};

/// The reserved wing name that grants apply globally. A grant on `*` applies to
/// every wing at its level; `*:admin` is a global admin (token management).
pub const GLOBAL_WING: &str = "*";

/// Permission level a token holds on a wing. Ordering matters: `Admin > Write > Read`,
/// derived from declaration order, so `level >= GrantLevel::Write` answers "can write?".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GrantLevel {
    Read,
    Write,
    Admin,
}

impl GrantLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            GrantLevel::Read => "read",
            GrantLevel::Write => "write",
            GrantLevel::Admin => "admin",
        }
    }

    /// Parse a level name. Returns `None` for unrecognised input so callers can
    /// reject a bad grant spec rather than silently downgrading.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "read" => Some(GrantLevel::Read),
            "write" => Some(GrantLevel::Write),
            "admin" => Some(GrantLevel::Admin),
            _ => None,
        }
    }
}

/// A stored access token. The plaintext secret is never persisted — only its hash —
/// and is shown to the operator exactly once at creation time.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AccessToken {
    pub id: i64,
    pub label: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    /// Soft revoke: set, never deleted, so the audit trail is preserved.
    pub revoked_at: Option<String>,
}

impl AccessToken {
    pub fn is_revoked(&self) -> bool {
        self.revoked_at.is_some()
    }
}

/// A single `(wing, level)` grant attached to a token.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Grant {
    pub wing: String,
    pub level: GrantLevel,
}

/// The resolved authority of an authenticated request: which wings it may touch and
/// at what level. Built by `SqliteStorage::resolve_scope` from a presented secret.
///
/// All scoping decisions go through this type so there is one place that decides
/// visibility — no handler hand-rolls its own check.
#[derive(Debug, Clone)]
pub struct AccessScope {
    pub token_id: i64,
    pub label: String,
    /// wing name → highest granted level for that wing.
    grants: HashMap<String, GrantLevel>,
}

impl AccessScope {
    pub fn new(token_id: i64, label: String, grants: Vec<Grant>) -> Self {
        let mut map: HashMap<String, GrantLevel> = HashMap::new();
        for g in grants {
            // Keep the strongest level if a wing is granted more than once.
            map.entry(g.wing)
                .and_modify(|cur| {
                    if g.level > *cur {
                        *cur = g.level;
                    }
                })
                .or_insert(g.level);
        }
        AccessScope { token_id, label, grants: map }
    }

    /// Effective level on `wing` = max of the wing-specific grant and any global (`*`) grant.
    pub fn level_for(&self, wing: &str) -> Option<GrantLevel> {
        let specific = self.grants.get(wing).copied();
        let global = self.grants.get(GLOBAL_WING).copied();
        specific.max(global)
    }

    pub fn can_read(&self, wing: &str) -> bool {
        self.level_for(wing).is_some()
    }

    pub fn can_write(&self, wing: &str) -> bool {
        matches!(self.level_for(wing), Some(l) if l >= GrantLevel::Write)
    }

    pub fn can_admin(&self, wing: &str) -> bool {
        matches!(self.level_for(wing), Some(GrantLevel::Admin))
    }

    /// True if this token may manage tokens at all (a global `*:admin` grant).
    pub fn is_global_admin(&self) -> bool {
        matches!(self.grants.get(GLOBAL_WING), Some(GrantLevel::Admin))
    }

    /// Drop every wing the token cannot read from `names`. Used to scope list endpoints
    /// so out-of-scope wings never appear in a response.
    pub fn retain_readable(&self, names: &mut Vec<String>) {
        names.retain(|n| self.can_read(n));
    }
}

/// Generate a fresh 256-bit token secret, hex-encoded (64 chars). Shown once.
pub fn generate_token_secret() -> String {
    let mut buf = [0u8; 32];
    // OS CSPRNG. If it ever fails we cannot safely mint a credential, so panic
    // rather than emit a low-entropy token.
    getrandom::getrandom(&mut buf).expect("OS RNG unavailable — cannot mint token");
    to_hex(&buf)
}

/// SHA-256 of the secret, hex-encoded — what gets stored and compared. The secret is
/// already high-entropy random, so a fast hash is sufficient to stop DB-read replay
/// without the cost of a password KDF.
pub fn hash_secret(secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    to_hex(&hasher.finalize())
}

fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_level_ordering() {
        assert!(GrantLevel::Admin > GrantLevel::Write);
        assert!(GrantLevel::Write > GrantLevel::Read);
    }

    #[test]
    fn scope_enforces_per_wing_levels() {
        let scope = AccessScope::new(
            1,
            "eng".into(),
            vec![Grant { wing: "engineering".into(), level: GrantLevel::Write }],
        );
        assert!(scope.can_read("engineering"));
        assert!(scope.can_write("engineering"));
        assert!(!scope.can_admin("engineering"));
        // No grant on `legal` ⇒ invisible.
        assert!(!scope.can_read("legal"));
        assert!(!scope.can_write("legal"));
    }

    #[test]
    fn global_grant_applies_everywhere() {
        let scope = AccessScope::new(
            2,
            "root".into(),
            vec![Grant { wing: GLOBAL_WING.into(), level: GrantLevel::Admin }],
        );
        assert!(scope.can_read("anything"));
        assert!(scope.can_write("anything"));
        assert!(scope.can_admin("anything"));
        assert!(scope.is_global_admin());
    }

    #[test]
    fn retain_readable_drops_out_of_scope_wings() {
        let scope = AccessScope::new(
            3,
            "eng".into(),
            vec![Grant { wing: "engineering".into(), level: GrantLevel::Read }],
        );
        let mut wings = vec!["engineering".to_string(), "legal".to_string()];
        scope.retain_readable(&mut wings);
        assert_eq!(wings, vec!["engineering".to_string()]);
    }

    #[test]
    fn hash_is_stable_and_secret_is_random() {
        let s1 = generate_token_secret();
        let s2 = generate_token_secret();
        assert_ne!(s1, s2);
        assert_eq!(s1.len(), 64);
        assert_eq!(hash_secret(&s1), hash_secret(&s1));
        assert_ne!(hash_secret(&s1), hash_secret(&s2));
    }
}
