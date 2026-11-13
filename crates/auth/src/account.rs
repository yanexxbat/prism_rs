//! `accounts.json`, format version 3. Ported from `AccountData` and `AccountList`.

use crate::session::{uuid_from_username, AuthSession};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// A token with its lifetime. `Debug` is not derived so tokens cannot end up in a log.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Token {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iat: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<i64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub token: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub refresh_token: String,
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

impl Token {
    /// Tokens with nothing in them are left out of the file.
    fn is_empty(&self) -> bool {
        self.token.is_empty() && self.refresh_token.is_empty() && self.extra.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum AccountType {
    #[serde(rename = "MSA")]
    Msa,
    Offline,
}

/// Skin and capes are kept as read, so data the port does not use survives a save.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cape: Option<String>,
    #[serde(default)]
    pub capes: Vec<Value>,
    pub id: String,
    pub name: String,
    pub skin: Value,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entitlement {
    pub can_play_minecraft: bool,
    pub owns_minecraft: bool,
}

/// What the last login or refresh said about the account. Not stored.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum AccountState {
    #[default]
    Unchecked,
    Online,
    /// The login servers could not be reached.
    Offline,
    Errored(String),
    /// The refresh token was rejected; the user has to log in again.
    Expired,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Account {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entitlement: Option<Entitlement>,
    #[serde(default, skip_serializing_if = "Token::is_empty")]
    pub msa: Token,
    #[serde(rename = "msa-client-id", default, skip_serializing_if = "String::is_empty")]
    pub msa_client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<Profile>,
    #[serde(rename = "type")]
    pub kind: AccountType,
    #[serde(default, skip_serializing_if = "Token::is_empty")]
    pub utoken: Token,
    #[serde(rename = "xrp-mc", default, skip_serializing_if = "Token::is_empty")]
    pub xrp_mc: Token,
    #[serde(default, skip_serializing_if = "Token::is_empty")]
    pub ygg: Token,

    #[serde(skip)]
    pub state: AccountState,
}

impl Account {
    pub fn new_msa(client_id: &str) -> Self {
        Account {
            active: false,
            entitlement: None,
            msa: Token::default(),
            msa_client_id: client_id.to_string(),
            profile: None,
            kind: AccountType::Msa,
            utoken: Token::default(),
            xrp_mc: Token::default(),
            ygg: Token::default(),
            state: AccountState::Unchecked,
        }
    }

    /// Ported from `MinecraftAccount::createOffline`.
    pub fn new_offline(username: &str) -> Self {
        let extra = Map::from_iter([
            ("userName".to_string(), Value::from(username)),
            ("clientToken".to_string(), Value::from(uuid::Uuid::new_v4().simple().to_string())),
        ]);
        Account {
            kind: AccountType::Offline,
            msa_client_id: String::new(),
            ygg: Token { iat: Some(now()), token: "0".into(), extra, ..Token::default() },
            profile: Some(Profile {
                cape: None,
                capes: Vec::new(),
                id: uuid_from_username(username),
                name: username.to_string(),
                skin: serde_json::json!({ "id": "", "url": "", "variant": "" }),
            }),
            ..Account::new_msa("")
        }
    }

    pub fn profile_id(&self) -> &str {
        self.profile.as_ref().map(|p| p.id.as_str()).unwrap_or_default()
    }

    pub fn profile_name(&self) -> &str {
        self.profile.as_ref().map(|p| p.name.as_str()).filter(|n| !n.is_empty()).unwrap_or("No Minecraft profile")
    }

    pub fn owns_minecraft(&self) -> bool {
        self.kind == AccountType::Msa && self.entitlement.is_some_and(|e| e.owns_minecraft)
    }

    /// Ported from `MinecraftAccount::shouldRefresh`: refresh when the game token
    /// runs out within twelve hours. A fresh token is valid for a day.
    pub fn should_refresh(&self) -> bool {
        if self.kind != AccountType::Msa || self.state == AccountState::Unchecked {
            return self.kind == AccountType::Msa;
        }
        let expires = self.ygg.exp.or(self.ygg.iat.map(|iat| iat + 24 * 3600)).unwrap_or(0);
        expires - now() < 12 * 3600
    }

    /// Ported from `MinecraftAccount::fillSession` and `AuthSession::MakeOffline`.
    /// `offline_name` is set for an offline launch and replaces the token and the name.
    pub fn session(&self, offline_name: Option<&str>) -> AuthSession {
        let player_name = self.profile_name().to_string();
        let uuid = match self.profile_id() {
            "" => uuid_from_username(&player_name),
            id => id.to_string(),
        };
        let user_type = if self.kind == AccountType::Msa { "msa" } else { "offline" }.to_string();
        match offline_name {
            Some(name) => AuthSession {
                player_name: name.to_string(),
                uuid,
                access_token: "0".into(),
                session: "-".into(),
                user_type,
                offline: true,
            },
            None => AuthSession {
                player_name,
                uuid,
                session: if self.ygg.token.is_empty() {
                    "-".into()
                } else {
                    format!("token:{}:{}", self.ygg.token, self.profile_id())
                },
                access_token: self.ygg.token.clone(),
                user_type,
                offline: false,
            },
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccountFile {
    accounts: Vec<Account>,
    format_version: i64,
}

pub struct AccountList {
    path: PathBuf,
    pub accounts: Vec<Account>,
}

impl AccountList {
    /// A missing file is an empty list. A file in another format is moved to
    /// `accounts-old.json`, as the original does, so nothing is overwritten.
    pub fn load(path: &Path) -> Result<Self> {
        let empty = AccountList { path: path.to_path_buf(), accounts: Vec::new() };
        if !path.exists() {
            return Ok(empty);
        }
        let io = |source| Error::Io { path: path.to_path_buf(), source };
        let text = std::fs::read_to_string(path).map_err(io)?;
        match serde_json::from_str::<AccountFile>(&text) {
            Ok(file) if file.format_version == 3 => Ok(AccountList { accounts: file.accounts, ..empty }),
            _ => {
                tracing::warn!("accounts.json has an unknown format; it is renamed to accounts-old.json");
                std::fs::rename(path, path.with_file_name("accounts-old.json")).map_err(io)?;
                Ok(empty)
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let file = AccountFile { accounts: self.accounts.clone(), format_version: 3 };
        let text = serde_json::to_string_pretty(&file).map_err(|e| Error::Protocol(e.to_string()))?;
        let io = |source| Error::Io { path: self.path.clone(), source };
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(io)?;
        std::fs::rename(&tmp, &self.path).map_err(io)
    }

    pub fn default_account(&self) -> Option<&Account> {
        self.accounts.iter().find(|a| a.active)
    }

    pub fn set_default(&mut self, index: usize) {
        for (i, account) in self.accounts.iter_mut().enumerate() {
            account.active = i == index;
        }
    }

    /// Adds an account, replacing one with the same type and profile id.
    /// The first account becomes the default.
    pub fn add(&mut self, mut account: Account) {
        let same = |a: &Account| a.kind == account.kind && a.profile_id() == account.profile_id();
        match self.accounts.iter().position(same) {
            Some(index) => {
                account.active = self.accounts[index].active;
                self.accounts[index] = account;
            }
            None => {
                account.active = self.accounts.is_empty();
                self.accounts.push(account);
            }
        }
    }

    /// Offline accounts need a Microsoft account that owns the game. Ported from
    /// `AccountList::anyAccountIsValid` and `AccountListPage::on_actionAddOffline_triggered`.
    pub fn add_offline(&mut self, username: &str) -> Result<()> {
        if !self.accounts.iter().any(Account::owns_minecraft) {
            return Err(Error::Refused(
                "You must add a Microsoft account that owns Minecraft before you can add an offline account.".into(),
            ));
        }
        self.add(Account::new_offline(username));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape Prism Launcher writes, with made-up values.
    const PRISM_FILE: &str = r#"{
        "accounts": [
            {
                "active": true,
                "entitlement": { "canPlayMinecraft": true, "ownsMinecraft": true },
                "msa": { "exp": 1790000000, "extra": { "scope": "XboxLive.signin" }, "iat": 1789990000, "refresh_token": "R", "token": "M" },
                "msa-client-id": "11111111-2222-3333-4444-555555555555",
                "profile": {
                    "capes": [ { "alias": "Migrator", "id": "c1", "url": "https://textures.minecraft.net/texture/x" } ],
                    "id": "0123456789abcdef0123456789abcdef",
                    "name": "Player",
                    "skin": { "data": "iVBORw0KGgo=", "id": "s1", "url": "https://textures.minecraft.net/texture/y", "variant": "CLASSIC" }
                },
                "type": "MSA",
                "utoken": { "exp": 1790000000, "extra": { "uhs": "123" }, "iat": 1789990000, "token": "U" },
                "xrp-mc": { "exp": 1790000000, "extra": { "uhs": "123" }, "iat": 1789990000, "token": "X" },
                "ygg": { "exp": 1790000000, "iat": 1789990000, "token": "Y" }
            },
            {
                "profile": { "capes": [], "id": "b50ad385829d3141a2167e7d7539ba7f", "name": "Notch", "skin": { "id": "", "url": "", "variant": "" } },
                "type": "Offline",
                "ygg": { "extra": { "clientToken": "c", "userName": "Notch" }, "iat": 1789990000, "token": "0" }
            }
        ],
        "formatVersion": 3
    }"#;

    #[test]
    fn prism_file_round_trips_without_loss() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("accounts.json");
        std::fs::write(&path, PRISM_FILE).expect("write");

        let list = AccountList::load(&path).expect("load");
        assert_eq!(list.accounts.len(), 2);
        assert!(list.accounts[0].owns_minecraft());
        assert_eq!(list.default_account().map(Account::profile_name), Some("Player"));
        list.save().expect("save");

        let before: Value = serde_json::from_str(PRISM_FILE).expect("json");
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("json");
        assert_eq!(before, after);
    }

    #[test]
    fn sessions_follow_the_original() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("accounts.json");
        std::fs::write(&path, PRISM_FILE).expect("write");
        let list = AccountList::load(&path).expect("load");

        let online = list.accounts[0].session(None);
        assert_eq!(online.session, "token:Y:0123456789abcdef0123456789abcdef");
        assert_eq!((online.access_token.as_str(), online.user_type.as_str(), online.offline), ("Y", "msa", false));

        // An offline launch keeps the profile id but gives up the token.
        let offline = list.accounts[0].session(Some("Guest"));
        assert_eq!((offline.player_name.as_str(), offline.access_token.as_str(), offline.session.as_str()), ("Guest", "0", "-"));
        assert_eq!(offline.uuid, online.uuid);
        assert!(offline.offline);

        let offline_account = list.accounts[1].session(None);
        assert_eq!(offline_account.user_type, "offline");
        assert_eq!(offline_account.session, "token:0:b50ad385829d3141a2167e7d7539ba7f");
        assert!(!offline_account.offline, "an offline account still launches online and downloads files");
    }

    #[test]
    fn offline_account_needs_an_owner() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut list = AccountList::load(&dir.path().join("accounts.json")).expect("load");
        assert!(matches!(list.add_offline("Steve"), Err(Error::Refused(_))));

        let mut owner = Account::new_msa("client");
        owner.entitlement = Some(Entitlement { can_play_minecraft: true, owns_minecraft: true });
        list.add(owner);
        list.add_offline("Steve").expect("allowed now");
        assert_eq!(list.accounts[1].profile_id(), uuid_from_username("Steve"));
        assert!(list.accounts[0].active && !list.accounts[1].active);

        // Adding the same name again replaces the entry.
        list.add_offline("Steve").expect("allowed");
        assert_eq!(list.accounts.len(), 2);
        list.set_default(1);
        assert_eq!(list.default_account().map(Account::profile_name), Some("Steve"));
    }

    #[test]
    fn unknown_format_is_set_aside() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("accounts.json");
        std::fs::write(&path, r#"{"formatVersion":2,"accounts":[]}"#).expect("write");
        let list = AccountList::load(&path).expect("load");
        assert!(list.accounts.is_empty());
        assert!(dir.path().join("accounts-old.json").is_file() && !path.exists());
    }

    #[test]
    fn refresh_is_due_twelve_hours_before_expiry() {
        let mut account = Account::new_msa("client");
        assert!(account.should_refresh(), "never checked in this session");
        account.state = AccountState::Online;
        account.ygg = Token { iat: Some(now()), exp: Some(now() + 24 * 3600), token: "Y".into(), ..Token::default() };
        assert!(!account.should_refresh());
        account.ygg.exp = Some(now() + 11 * 3600);
        assert!(account.should_refresh());
        assert!(!Account::new_offline("Steve").should_refresh());
    }
}
