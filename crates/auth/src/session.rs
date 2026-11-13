//! What a launch needs to know about the player. Ported from `AuthSession`.

use md5::{Digest, Md5};

#[derive(Clone, Debug, PartialEq)]
pub struct AuthSession {
    pub player_name: String,
    /// Profile id without dashes.
    pub uuid: String,
    pub access_token: String,
    /// Legacy session id: `token:<access token>:<profile id>`, or `-` when offline.
    pub session: String,
    /// `msa` or `offline`.
    pub user_type: String,
    /// Offline launches skip every download and use what is on disk.
    pub offline: bool,
}

impl AuthSession {
    /// Ported from `AuthSession::MakeOffline`: no token, a name-derived profile id.
    pub fn offline(player_name: &str) -> Self {
        AuthSession {
            player_name: player_name.to_string(),
            uuid: uuid_from_username(player_name),
            access_token: "0".into(),
            session: "-".into(),
            user_type: "offline".into(),
            offline: true,
        }
    }

    /// Text that must never reach a log, with its replacement.
    /// Ported from `MinecraftInstance::createCensorFilterFromSession`.
    pub fn censor_filter(&self) -> Vec<(String, String)> {
        let mut filter = Vec::new();
        if self.session != "-" {
            filter.push((self.session.clone(), "<SESSION ID>".to_string()));
        }
        if self.access_token != "0" {
            filter.push((self.access_token.clone(), "<ACCESS TOKEN>".to_string()));
        }
        filter.push((self.uuid.clone(), "<PROFILE ID>".to_string()));
        filter.retain(|(secret, _)| !secret.trim().is_empty());
        filter
    }
}

/// Java's `UUID.nameUUIDFromBytes("OfflinePlayer:<name>")`, as 32 hex digits.
pub fn uuid_from_username(username: &str) -> String {
    let mut digest: [u8; 16] = Md5::digest(format!("OfflinePlayer:{username}")).into();
    digest[6] = (digest[6] & 0x0f) | 0x30; // version 3
    digest[8] = (digest[8] & 0x3f) | 0x80; // IETF variant
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_uuid_matches_java() {
        // UUID.nameUUIDFromBytes("OfflinePlayer:Notch".getBytes(UTF_8)) in Java
        assert_eq!(uuid_from_username("Notch"), "b50ad385829d3141a2167e7d7539ba7f");
    }

    #[test]
    fn offline_session_has_nothing_secret_but_the_id() {
        let session = AuthSession::offline("Steve");
        assert_eq!(session.censor_filter(), vec![(session.uuid.clone(), "<PROFILE ID>".to_string())]);
    }
}
