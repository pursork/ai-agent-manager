//! Codex account fingerprinting: replicates codex-skill's proven algorithm
//! for deriving a stable identifier from `auth.json`'s JWTs -- read
//! directly from the real, currently-in-use source
//! (`C:\Users\16500\.codex\codex-interface-manager\src\account.ps1`'s
//! `Get-AccountInfo` and `common.ps1`'s `Decode-JwtPayload`/`Get-Sha256Hex`,
//! not reconstructed from memory) rather than invented fresh.
//!
//! This is what makes `docs/04-webdav-sync-security.md` §4.10's Codex
//! account-credential blob key stable across token refresh: the fingerprint
//! is derived from identity *claims* embedded in the JWTs, not from the
//! token strings themselves (which rotate on every refresh).

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub enum FingerprintError {
    InvalidJson(&'static str),
    MissingField(&'static str),
    NotAJwt(&'static str),
    InvalidBase64(&'static str),
    /// Neither an email nor a user id/subject claim was found in either
    /// JWT -- codex-skill treats this as unsafe to fingerprint, and so do
    /// we (matches its `Stop-WithError '...无法安全区分账号。'`).
    NoStableIdentity,
}

impl fmt::Display for FingerprintError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FingerprintError::InvalidJson(field) => write!(f, "{field} is not valid JSON"),
            FingerprintError::MissingField(field) => write!(f, "auth.json is missing '{field}'"),
            FingerprintError::NotAJwt(field) => write!(f, "{field} is not a three-part JWT"),
            FingerprintError::InvalidBase64(field) => {
                write!(f, "{field}'s JWT payload is not valid base64url")
            }
            FingerprintError::NoStableIdentity => write!(
                f,
                "neither an email nor a user id/subject claim was found in auth.json's JWTs -- \
                 cannot safely derive a stable account fingerprint"
            ),
        }
    }
}

impl Error for FingerprintError {}

const AUTH_NAMESPACE: &str = "https://api.openai.com/auth";
const PROFILE_NAMESPACE: &str = "https://api.openai.com/profile";

/// Hand-rolled to match `aam-sync/src/backend.rs`'s `basic_auth_base64`
/// style -- a single-call-site decode doesn't justify a new dependency.
fn base64url_decode(input: &str, field_name: &'static str) -> Result<Vec<u8>, FingerprintError> {
    fn value(c: u8, field_name: &'static str) -> Result<u8, FingerprintError> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'-' => Ok(62),
            b'_' => Ok(63),
            _ => Err(FingerprintError::InvalidBase64(field_name)),
        }
    }

    let cleaned: &str = input.trim_end_matches('=');
    let chars: Vec<u8> = cleaned.bytes().collect();
    if chars.len() % 4 == 1 {
        return Err(FingerprintError::InvalidBase64(field_name));
    }

    let mut out = Vec::with_capacity(chars.len() * 3 / 4);
    for chunk in chars.chunks(4) {
        let vals: Vec<u8> = chunk
            .iter()
            .map(|&c| value(c, field_name))
            .collect::<Result<_, _>>()?;
        let n = vals.len();
        let b0 = vals[0];
        let b1 = *vals.get(1).unwrap_or(&0);
        out.push((b0 << 2) | (b1 >> 4));
        if n > 2 {
            let b2 = vals[2];
            out.push((b1 << 4) | (b2 >> 2));
        }
        if n > 3 {
            let b2 = vals[2];
            let b3 = vals[3];
            out.push((b2 << 6) | b3);
        }
    }
    Ok(out)
}

fn decode_jwt_payload(token: &str, field_name: &'static str) -> Result<Value, FingerprintError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(FingerprintError::NotAJwt(field_name));
    }
    let bytes = base64url_decode(parts[1], field_name)?;
    serde_json::from_slice(&bytes).map_err(|_| FingerprintError::InvalidJson(field_name))
}

fn get_str<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// First non-empty candidate wins. Simplification vs. codex-skill's
/// `Get-ConsistentValue`, which errors if redundant claim sources
/// disagree -- that cross-check matters for an account-switcher actively
/// guarding against a tampered/corrupted credential file; this crate only
/// needs *a* stable identity value, and disagreement across sources here
/// would be extremely unusual (they all originate from the same OAuth
/// exchange).
fn first_non_empty(candidates: &[Option<&str>]) -> String {
    candidates
        .iter()
        .find_map(|c| c.filter(|s| !s.is_empty()))
        .unwrap_or_default()
        .to_string()
}

/// The identity a fingerprint is derived from, exposed in full so callers
/// (`account_sync.rs`'s `accounts.json.age` catalog) can show a human a
/// hint of whose account a fingerprint refers to, without re-parsing
/// `auth.json` themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountIdentity {
    pub fingerprint: String,
    /// Empty if no email claim was present in either JWT (codex-skill
    /// allows this as long as a user id/subject was found instead).
    pub email: String,
    pub account_id: String,
}

/// Computes the stable Codex account fingerprint (`identity.fingerprint`)
/// plus the email/account_id claims it was derived from, from raw
/// `auth.json` bytes: the fingerprint is SHA-256 of
/// `lower(user_id)|lower(subject)|lower(email)|account_id` (note:
/// `account_id` itself is *not* lowercased, matching codex-skill),
/// truncated to the first 20 hex characters.
pub fn extract_identity(auth_json_bytes: &[u8]) -> Result<AccountIdentity, FingerprintError> {
    let root: Value =
        serde_json::from_slice(auth_json_bytes).map_err(|_| FingerprintError::InvalidJson("auth.json"))?;
    let token_source = root.get("tokens").unwrap_or(&root);

    let id_token = get_str(token_source, "id_token").ok_or(FingerprintError::MissingField("id_token"))?;
    let access_token = get_str(token_source, "access_token");

    let id_claims = decode_jwt_payload(id_token, "id_token")?;
    let access_claims = match access_token {
        Some(t) => Some(decode_jwt_payload(t, "access_token")?),
        None => None,
    };

    let id_auth = id_claims.get(AUTH_NAMESPACE);
    let access_auth = access_claims.as_ref().and_then(|c| c.get(AUTH_NAMESPACE));
    let access_profile = access_claims.as_ref().and_then(|c| c.get(PROFILE_NAMESPACE));

    let account_id = {
        let v = first_non_empty(&[
            get_str(token_source, "account_id"),
            get_str(&root, "account_id"),
            id_auth.and_then(|a| get_str(a, "chatgpt_account_id")),
            access_auth.and_then(|a| get_str(a, "chatgpt_account_id")),
        ]);
        if v.is_empty() {
            return Err(FingerprintError::MissingField("account_id"));
        }
        v
    };

    let email = first_non_empty(&[
        get_str(&root, "email"),
        get_str(&id_claims, "email"),
        access_profile.and_then(|p| get_str(p, "email")),
    ]);

    let mut user_id = first_non_empty(&[
        id_auth.and_then(|a| get_str(a, "chatgpt_user_id")),
        id_auth.and_then(|a| get_str(a, "user_id")),
        access_auth.and_then(|a| get_str(a, "chatgpt_user_id")),
        access_auth.and_then(|a| get_str(a, "user_id")),
    ]);

    let subject = first_non_empty(&[
        get_str(&id_claims, "sub"),
        access_claims.as_ref().and_then(|c| get_str(c, "sub")),
    ]);

    if user_id.is_empty() {
        user_id = subject.clone();
    }
    if email.is_empty() && user_id.is_empty() {
        return Err(FingerprintError::NoStableIdentity);
    }

    let identity = format!(
        "{}|{}|{}|{}",
        user_id.to_lowercase(),
        subject.to_lowercase(),
        email.to_lowercase(),
        account_id
    );

    let hash = Sha256::digest(identity.as_bytes());
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    Ok(AccountIdentity {
        fingerprint: hex[..20].to_string(),
        email,
        account_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base64url_encode(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b0 = chunk[0];
            let b1 = *chunk.get(1).unwrap_or(&0);
            let b2 = *chunk.get(2).unwrap_or(&0);
            out.push(ALPHABET[(b0 >> 2) as usize] as char);
            out.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);
            if chunk.len() > 1 {
                out.push(ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
            }
            if chunk.len() > 2 {
                out.push(ALPHABET[(b2 & 0x3f) as usize] as char);
            }
        }
        out
    }

    fn make_jwt(payload_json: &str) -> String {
        let header = base64url_encode(br#"{"alg":"none"}"#);
        let payload = base64url_encode(payload_json.as_bytes());
        format!("{header}.{payload}.fakesignature")
    }

    fn make_auth_json(id_claims: &str, access_claims: &str, account_id: &str) -> Vec<u8> {
        let id_token = make_jwt(id_claims);
        let access_token = make_jwt(access_claims);
        serde_json::json!({
            "tokens": {
                "id_token": id_token,
                "access_token": access_token,
                "refresh_token": "rt-does-not-matter",
                "account_id": account_id,
            },
            "last_refresh": "2026-08-08T12:00:00Z",
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn produces_a_20_hex_char_fingerprint() {
        let auth = make_auth_json(
            r#"{"sub":"user-abc","email":"person@example.com","https://api.openai.com/auth":{"chatgpt_account_id":"acct-1","chatgpt_user_id":"user-abc"}}"#,
            r#"{"sub":"user-abc"}"#,
            "acct-1",
        );
        let fp = extract_identity(&auth).unwrap().fingerprint;
        assert_eq!(fp.len(), 20);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn is_stable_across_simulated_token_refresh() {
        // Same identity claims, but the JWTs themselves differ (as they
        // would after a real refresh -- new exp/iat, same sub/email).
        let auth1 = make_auth_json(
            r#"{"sub":"user-abc","email":"person@example.com","iat":1000,"https://api.openai.com/auth":{"chatgpt_account_id":"acct-1"}}"#,
            r#"{"sub":"user-abc","iat":1000}"#,
            "acct-1",
        );
        let auth2 = make_auth_json(
            r#"{"sub":"user-abc","email":"person@example.com","iat":9999,"https://api.openai.com/auth":{"chatgpt_account_id":"acct-1"}}"#,
            r#"{"sub":"user-abc","iat":9999}"#,
            "acct-1",
        );
        assert_eq!(
            extract_identity(&auth1).unwrap().fingerprint,
            extract_identity(&auth2).unwrap().fingerprint
        );
    }

    #[test]
    fn different_accounts_produce_different_fingerprints() {
        let auth_a = make_auth_json(r#"{"sub":"user-a","email":"a@example.com"}"#, "{}", "acct-a");
        let auth_b = make_auth_json(r#"{"sub":"user-b","email":"b@example.com"}"#, "{}", "acct-b");
        assert_ne!(
            extract_identity(&auth_a).unwrap().fingerprint,
            extract_identity(&auth_b).unwrap().fingerprint
        );
    }

    #[test]
    fn email_case_does_not_affect_fingerprint() {
        let auth_lower =
            make_auth_json(r#"{"sub":"user-a","email":"person@example.com"}"#, "{}", "acct-a");
        let auth_upper =
            make_auth_json(r#"{"sub":"user-a","email":"PERSON@EXAMPLE.COM"}"#, "{}", "acct-a");
        assert_eq!(
            extract_identity(&auth_lower).unwrap().fingerprint,
            extract_identity(&auth_upper).unwrap().fingerprint
        );
    }

    #[test]
    fn missing_email_and_user_id_and_subject_errors() {
        let auth = make_auth_json("{}", "{}", "acct-a");
        let err = extract_identity(&auth).unwrap_err();
        assert!(matches!(err, FingerprintError::NoStableIdentity));
    }

    #[test]
    fn missing_account_id_errors() {
        let id_token = make_jwt(r#"{"sub":"user-a","email":"a@example.com"}"#);
        let access_token = make_jwt("{}");
        let auth = serde_json::json!({
            "tokens": {
                "id_token": id_token,
                "access_token": access_token,
                "refresh_token": "rt",
            }
        })
        .to_string()
        .into_bytes();
        let err = extract_identity(&auth).unwrap_err();
        assert!(matches!(err, FingerprintError::MissingField("account_id")));
    }

    #[test]
    fn non_jwt_id_token_errors() {
        let auth = serde_json::json!({
            "tokens": {
                "id_token": "not-a-jwt",
                "access_token": "also-not-a-jwt",
                "refresh_token": "rt",
                "account_id": "acct-1",
            }
        })
        .to_string()
        .into_bytes();
        let err = extract_identity(&auth).unwrap_err();
        assert!(matches!(err, FingerprintError::NotAJwt("id_token")));
    }
}
