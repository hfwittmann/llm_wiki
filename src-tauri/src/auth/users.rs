use std::collections::HashMap;
use std::path::Path;

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum UsersError {
    #[error("users.toml not found: {0}")]
    NotFound(String),
    #[error("users.toml could not be read: {0}")]
    Io(#[from] std::io::Error),
    #[error("users.toml is malformed: {0}")]
    Malformed(String),
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("password hashing failed: {0}")]
    Hash(String),
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: String,
    pub username: String,
}

#[derive(Debug, Clone, Deserialize)]
struct UserRecord {
    password_hash: String,
}

#[derive(Debug, Clone, Deserialize)]
struct UsersFile {
    #[serde(default)]
    users: HashMap<String, UserRecord>,
}

#[derive(Debug, Clone)]
pub struct Users {
    by_id: HashMap<String, UserRecord>,
    display_names: HashMap<String, String>,
    // A pre-computed sentinel hash used to keep the unknown-user branch
    // of verify_password running exactly one argon2 KDF — same cost as the
    // known-user branch, so timing cannot distinguish the two cases.
    sentinel_hash: String,
}

impl Users {
    pub fn load(path: &Path) -> Result<Self, UsersError> {
        let raw = std::fs::read_to_string(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                UsersError::NotFound(path.display().to_string())
            } else {
                UsersError::Io(e)
            }
        })?;
        let parsed: UsersFile = toml::from_str(&raw)
            .map_err(|e| UsersError::Malformed(e.to_string()))?;

        let mut by_id = HashMap::new();
        let mut display_names = HashMap::new();
        for (name, record) in parsed.users {
            let id = name.to_lowercase();
            display_names.insert(id.clone(), name);
            by_id.insert(id, record);
        }

        // Compute the sentinel hash up front. We tolerate failure here (an
        // empty sentinel would defeat the timing protection, but argon2 hash
        // generation realistically can't fail with a constant input), and
        // log via debug_assert so a regression in dev surfaces loudly.
        let sentinel_hash = hash_password("__timing_oracle_sentinel__")
            .expect("argon2 hash of a constant input cannot fail");

        Ok(Users { by_id, display_names, sentinel_hash })
    }

    pub fn verify_password(&self, username: &str, plaintext: &str) -> Result<User, AuthError> {
        let id = username.to_lowercase();
        let record = match self.by_id.get(&id) {
            Some(r) => r,
            None => {
                // Unknown user: run a single argon2 verify against the
                // pre-computed sentinel so this branch costs the same as a
                // real verify. No second KDF, no timing oracle.
                let _ = PasswordHash::new(&self.sentinel_hash).and_then(|h| {
                    Argon2::default().verify_password(plaintext.as_bytes(), &h)
                });
                return Err(AuthError::InvalidCredentials);
            }
        };

        let parsed = PasswordHash::new(&record.password_hash)
            .map_err(|e| AuthError::Hash(e.to_string()))?;
        Argon2::default()
            .verify_password(plaintext.as_bytes(), &parsed)
            .map_err(|_| AuthError::InvalidCredentials)?;

        let username = self
            .display_names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| id.clone());
        Ok(User { id, username })
    }
}

pub fn hash_password(plaintext: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(plaintext.as_bytes(), &salt)
        .map_err(|e| AuthError::Hash(e.to_string()))?
        .to_string();
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_users_toml(dir: &TempDir, contents: &str) -> std::path::PathBuf {
        let path = dir.path().join("users.toml");
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn hash_then_verify_roundtrip() {
        let hash = hash_password("correct horse battery staple").unwrap();
        let dir = TempDir::new().unwrap();
        let path = write_users_toml(
            &dir,
            &format!(
                "[users.alice]\npassword_hash = \"{}\"\n",
                hash.replace('\\', "\\\\")
            ),
        );
        let users = Users::load(&path).unwrap();
        let user = users
            .verify_password("alice", "correct horse battery staple")
            .unwrap();
        assert_eq!(user.id, "alice");
    }

    #[test]
    fn rejects_wrong_password() {
        let hash = hash_password("right").unwrap();
        let dir = TempDir::new().unwrap();
        let path = write_users_toml(
            &dir,
            &format!("[users.alice]\npassword_hash = \"{}\"\n", hash),
        );
        let users = Users::load(&path).unwrap();
        let result = users.verify_password("alice", "wrong");
        assert!(matches!(result, Err(AuthError::InvalidCredentials)));
    }

    #[test]
    fn rejects_unknown_user_with_same_error_as_wrong_password() {
        let hash = hash_password("right").unwrap();
        let dir = TempDir::new().unwrap();
        let path = write_users_toml(
            &dir,
            &format!("[users.alice]\npassword_hash = \"{}\"\n", hash),
        );
        let users = Users::load(&path).unwrap();
        let result = users.verify_password("nobody", "anything");
        assert!(matches!(result, Err(AuthError::InvalidCredentials)));
    }

    #[test]
    fn username_is_case_insensitive_for_lookup() {
        let hash = hash_password("pw").unwrap();
        let dir = TempDir::new().unwrap();
        let path = write_users_toml(
            &dir,
            &format!("[users.Alice]\npassword_hash = \"{}\"\n", hash),
        );
        let users = Users::load(&path).unwrap();
        let user = users.verify_password("alice", "pw").unwrap();
        assert_eq!(user.id, "alice");
        assert_eq!(user.username, "Alice");
    }

    #[test]
    fn load_returns_not_found_for_missing_file() {
        let dir = TempDir::new().unwrap();
        let result = Users::load(&dir.path().join("does_not_exist.toml"));
        assert!(matches!(result, Err(UsersError::NotFound(_))));
    }

    #[test]
    fn load_returns_malformed_on_bad_toml() {
        let dir = TempDir::new().unwrap();
        let path = write_users_toml(&dir, "not = toml = at all");
        let result = Users::load(&path);
        assert!(matches!(result, Err(UsersError::Malformed(_))));
    }

    #[test]
    fn load_empty_file_returns_empty_users() {
        let dir = TempDir::new().unwrap();
        let path = write_users_toml(&dir, "");
        let users = Users::load(&path).unwrap();
        let result = users.verify_password("anyone", "pw");
        assert!(matches!(result, Err(AuthError::InvalidCredentials)));
    }

    #[test]
    fn corrupted_stored_hash_returns_hash_error() {
        let dir = TempDir::new().unwrap();
        // user record exists but the password_hash isn't a valid argon2 string
        let path = write_users_toml(
            &dir,
            "[users.alice]\npassword_hash = \"not-a-real-argon2-hash\"\n",
        );
        let users = Users::load(&path).unwrap();
        let result = users.verify_password("alice", "anything");
        assert!(
            matches!(result, Err(AuthError::Hash(_))),
            "expected AuthError::Hash for corrupted stored hash, got {:?}",
            result
        );
    }
}
