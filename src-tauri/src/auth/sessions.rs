use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};

const DEFAULT_SESSION_TTL_SECS: u64 = 60 * 60 * 24 * 30; // 30 days

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("sled error: {0}")]
    Sled(#[from] sled::Error),
    #[error("session serde error: {0}")]
    Serde(#[from] bincode::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionId(pub String);

impl SessionId {
    pub fn new() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        SessionId(URL_SAFE_NO_PAD.encode(bytes))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionRecord {
    user_id: String,
    expires_at_unix: u64,
}

#[derive(Clone)]
pub struct Sessions {
    db: sled::Db,
    ttl_secs: u64,
}

impl Sessions {
    pub fn open(path: &Path) -> Result<Self, SessionError> {
        let db = sled::open(path)?;
        Ok(Sessions { db, ttl_secs: DEFAULT_SESSION_TTL_SECS })
    }

    pub fn create(&self, user_id: &str) -> Result<SessionId, SessionError> {
        let sid = SessionId::new();
        let record = SessionRecord {
            user_id: user_id.to_string(),
            expires_at_unix: now_unix().saturating_add(self.ttl_secs),
        };
        let bytes = bincode::serialize(&record)?;
        self.db.insert(sid.as_str().as_bytes(), bytes)?;
        self.db.flush()?;
        Ok(sid)
    }

    pub fn lookup(&self, id: &str) -> Option<String> {
        let bytes = self.db.get(id.as_bytes()).ok().flatten()?;
        let record: SessionRecord = bincode::deserialize(&bytes).ok()?;
        if record.expires_at_unix <= now_unix() {
            // Lazy expiry — fire-and-forget removal.
            let _ = self.db.remove(id.as_bytes());
            return None;
        }
        Some(record.user_id)
    }

    pub fn delete(&self, id: &str) -> Result<(), SessionError> {
        self.db.remove(id.as_bytes())?;
        self.db.flush()?;
        Ok(())
    }

    pub fn prune_expired(&self) -> Result<usize, SessionError> {
        let now = now_unix();
        let mut count = 0;
        let mut to_delete = Vec::new();
        for entry in self.db.iter() {
            let (k, v) = entry?;
            if let Ok(record) = bincode::deserialize::<SessionRecord>(&v) {
                if record.expires_at_unix <= now {
                    to_delete.push(k.to_vec());
                }
            }
        }
        for k in to_delete {
            self.db.remove(k)?;
            count += 1;
        }
        self.db.flush()?;
        Ok(count)
    }

    #[cfg(test)]
    pub(crate) fn with_ttl(mut self, ttl_secs: u64) -> Self {
        self.ttl_secs = ttl_secs;
        self
    }
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn open_sessions(dir: &TempDir) -> Sessions {
        Sessions::open(&dir.path().join("sessions")).unwrap()
    }

    #[test]
    fn create_then_lookup_returns_user_id() {
        let dir = TempDir::new().unwrap();
        let s = open_sessions(&dir);
        let sid = s.create("alice").unwrap();
        assert_eq!(s.lookup(sid.as_str()), Some("alice".to_string()));
    }

    #[test]
    fn lookup_for_unknown_session_returns_none() {
        let dir = TempDir::new().unwrap();
        let s = open_sessions(&dir);
        assert!(s.lookup("nonexistent").is_none());
    }

    #[test]
    fn delete_removes_session() {
        let dir = TempDir::new().unwrap();
        let s = open_sessions(&dir);
        let sid = s.create("bob").unwrap();
        s.delete(sid.as_str()).unwrap();
        assert!(s.lookup(sid.as_str()).is_none());
    }

    #[test]
    fn session_ids_are_unique() {
        let dir = TempDir::new().unwrap();
        let s = open_sessions(&dir);
        let a = s.create("alice").unwrap();
        let b = s.create("alice").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn sessions_survive_reopen() {
        let dir = TempDir::new().unwrap();
        let sid = {
            let s = open_sessions(&dir);
            s.create("carol").unwrap()
        };
        // sled dropped, reopen the same path
        let s2 = open_sessions(&dir);
        assert_eq!(s2.lookup(sid.as_str()), Some("carol".to_string()));
    }

    #[test]
    fn expired_sessions_are_not_returned_by_lookup() {
        let dir = TempDir::new().unwrap();
        let s = open_sessions(&dir).with_ttl(0); // expires immediately
        let sid = s.create("dave").unwrap();
        // sleep a moment to ensure clock advances past expiry
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(s.lookup(sid.as_str()).is_none());
    }

    #[test]
    fn prune_expired_removes_old_rows() {
        let dir = TempDir::new().unwrap();
        let s = open_sessions(&dir).with_ttl(0);
        for u in ["e", "f", "g"] {
            s.create(u).unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        let pruned = s.prune_expired().unwrap();
        assert_eq!(pruned, 3);
    }
}
