//! Microsoft login: device code → Xbox Live → XSTS → Minecraft services.
//! Ported from `AuthFlow` and `steps/{MSADeviceCodeStep, MSAStep (refresh), XboxUserStep,
//! XboxAuthorizationStep, LauncherLoginStep, EntitlementsStep, MinecraftProfileStep}`.
//!
//! Response bodies and tokens are never logged.

use crate::account::{now, Account, AccountState, Entitlement, Profile, Token};
use crate::{Error, Result};
use prism_net::Client;
use serde_json::{json, Map, Value};
use std::time::Duration;

const SCOPE: &str = "XboxLive.SignIn XboxLive.offline_access";

/// The servers of the flow. Tests point these at a local server.
#[derive(Clone, Debug)]
pub struct Endpoints {
    pub device_code: String,
    pub token: String,
    pub xbox_user: String,
    pub xsts: String,
    pub launcher_login: String,
    pub entitlements: String,
    pub profile: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            device_code: "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode".into(),
            token: "https://login.microsoftonline.com/consumers/oauth2/v2.0/token".into(),
            xbox_user: "https://user.auth.xboxlive.com/user/authenticate".into(),
            xsts: "https://xsts.auth.xboxlive.com/xsts/authorize".into(),
            launcher_login: "https://api.minecraftservices.com/launcher/login".into(),
            entitlements: "https://api.minecraftservices.com/entitlements/license".into(),
            profile: "https://api.minecraftservices.com/minecraft/profile".into(),
        }
    }
}

/// What the user needs to finish the login in a browser.
#[derive(Clone, Debug)]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    interval: u64,
    device_code: String,
}

fn text(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// Sends a request and returns the status with the body as JSON (`Null` when it is not JSON).
async fn send(request: reqwest::RequestBuilder, what: &str) -> Result<(reqwest::StatusCode, Value)> {
    let unreachable = |e: reqwest::Error| Error::Network(format!("{what}: {}", e.without_url()));
    let response = request.header("Accept", "application/json").send().await.map_err(unreachable)?;
    let status = response.status();
    let body = response.bytes().await.map_err(unreachable)?;
    Ok((status, serde_json::from_slice(&body).unwrap_or(Value::Null)))
}

/// 5xx means "try later"; 4xx means the request itself was refused.
fn check(status: reqwest::StatusCode, what: &str) -> Result<()> {
    match status {
        s if s.is_success() => Ok(()),
        s if s.is_server_error() => Err(Error::Network(format!("{what}: server answered {s}"))),
        s => Err(Error::Failed(format!("{what}: server answered {s}"))),
    }
}

pub async fn request_device_code(client: &Client, endpoints: &Endpoints, client_id: &str) -> Result<DeviceCode> {
    let what = "Device authorization failed";
    let form = [("client_id", client_id), ("scope", SCOPE)];
    let (_, body) = send(client.http().post(&endpoints.device_code).form(&form), what).await?;
    let (error, description) = (text(&body, "error"), text(&body, "error_description"));
    if !error.is_empty() || !description.is_empty() {
        return Err(Error::Failed(format!("{what}: {}", if description.is_empty() { error } else { description })));
    }
    let code = DeviceCode {
        user_code: text(&body, "user_code"),
        verification_uri: text(&body, "verification_uri"),
        expires_in: body.get("expires_in").and_then(Value::as_u64).unwrap_or(0),
        interval: body.get("interval").and_then(Value::as_u64).filter(|i| *i > 0).unwrap_or(5),
        device_code: text(&body, "device_code"),
    };
    if code.device_code.is_empty() || code.user_code.is_empty() || code.verification_uri.is_empty() || code.expires_in == 0 {
        return Err(Error::Failed(format!("{what}: required fields missing")));
    }
    Ok(code)
}

fn msa_token(body: &Value) -> Token {
    let issued = now();
    Token {
        iat: Some(issued),
        exp: body.get("expires_in").and_then(Value::as_i64).map(|seconds| issued + seconds),
        token: text(body, "access_token"),
        refresh_token: text(body, "refresh_token"),
        // The original stores the whole response here; that would duplicate both tokens.
        extra: Map::new(),
    }
}

/// Asks the token endpoint until the user has confirmed the code or it expires.
pub async fn poll_device_code(client: &Client, endpoints: &Endpoints, client_id: &str, code: &DeviceCode) -> Result<Token> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(code.expires_in);
    let mut interval = code.interval;
    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        if tokio::time::Instant::now() >= deadline {
            return Err(Error::Failed("Device Access failed: the code has expired".into()));
        }
        let form = [
            ("client_id", client_id),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", code.device_code.as_str()),
        ];
        // A request that fails on the way is simply tried again.
        let Ok((_, body)) = send(client.http().post(&endpoints.token).form(&form), "Device Access failed").await else {
            continue;
        };
        match text(&body, "error").as_str() {
            "authorization_pending" => continue,
            "slow_down" => interval += 5,
            "" if !text(&body, "access_token").is_empty() => return Ok(msa_token(&body)),
            "" => continue,
            error => {
                let description = text(&body, "error_description");
                let reason = if description.is_empty() { error.to_string() } else { description };
                return Err(Error::Failed(format!("Device Access failed: {reason}")));
            }
        }
    }
}

async fn refresh_msa_token(client: &Client, endpoints: &Endpoints, client_id: &str, account: &mut Account) -> Result<()> {
    if account.msa_client_id != client_id {
        return Err(Error::Expired("Microsoft user authentication failed - client identification has changed.".into()));
    }
    if account.msa.refresh_token.is_empty() {
        return Err(Error::Expired("Microsoft user authentication failed - refresh token is empty.".into()));
    }
    let what = "Microsoft user authentication failed";
    let form = [
        ("client_id", client_id),
        ("grant_type", "refresh_token"),
        ("refresh_token", account.msa.refresh_token.as_str()),
        ("scope", SCOPE),
    ];
    let (status, body) = send(client.http().post(&endpoints.token).form(&form), what).await?;
    if status.is_server_error() {
        return Err(Error::Network(format!("{what}: server answered {status}")));
    }
    if !status.is_success() || text(&body, "access_token").is_empty() {
        // A rejected refresh token cannot be repaired without the user.
        return Err(Error::Expired(format!("{what}: {}", text(&body, "error"))));
    }
    let mut token = msa_token(&body);
    if token.refresh_token.is_empty() {
        token.refresh_token = account.msa.refresh_token.clone();
    }
    account.msa = token;
    Ok(())
}

/// `{"Token": …, "DisplayClaims": {"xui": [{"uhs": …}]}}`
fn x_token(body: &Value) -> Option<Token> {
    let token = text(body, "Token");
    let uhs = body.get("DisplayClaims")?.get("xui")?.as_array()?.iter().find_map(|claim| claim.get("uhs")?.as_str())?;
    // shortcut: issue and expiry times of Xbox tokens are not parsed; nothing reads them. Upgrade if that changes.
    let extra = Map::from_iter([("uhs".to_string(), Value::from(uhs))]);
    (!token.is_empty()).then(|| Token { iat: Some(now()), token, extra, ..Token::default() })
}

/// Ported from `XboxAuthorizationStep::processSTSError`.
fn xsts_error(body: &Value) -> Option<String> {
    Some(
        match body.get("XErr")?.as_u64()? {
            2148916227 => "This Microsoft account was banned by Xbox for violating one or more Community Standards for Xbox and is unable to be used.",
            2148916229 => "This account is currently restricted and your guardian has not given you permission to play online.",
            2148916233 => "This Microsoft account does not have an Xbox Live profile. Buy the game on minecraft.net first.",
            2148916234 => "This Microsoft account has not accepted Xbox's Terms of Service. Please log in online and accept them.",
            2148916235 => "Xbox Live is not available in your country. You've been blocked.",
            2148916236 => "This Microsoft account requires proof of age to play. Please log in to login.live.com to provide proof of age.",
            2148916237 => "This Microsoft account has reached its limit for playtime. This account has been blocked from logging in.",
            2148916238 => "This Microsoft account is underaged and is not linked to a family.\n\nPlease set up your account according to help.minecraft.net.",
            other => return Some(format!("XSTS authentication ended with unrecognized error: {other}")),
        }
        .to_string(),
    )
}

/// Everything after the Microsoft token: Xbox user token, XSTS, Minecraft token, ownership, profile.
async fn finish(client: &Client, endpoints: &Endpoints, account: &mut Account) -> Result<()> {
    let http = client.http();
    let xbox_headers = |request: reqwest::RequestBuilder| request.header("x-xbl-contract-version", "1");

    let what = "Xbox user authentication failed";
    let body = json!({
        "Properties": { "AuthMethod": "RPS", "SiteName": "user.auth.xboxlive.com", "RpsTicket": format!("d={}", account.msa.token) },
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT"
    });
    let (status, response) = send(xbox_headers(http.post(&endpoints.xbox_user)).json(&body), what).await?;
    check(status, what)?;
    account.utoken = x_token(&response).ok_or_else(|| Error::Failed("Xbox user authentication response could not be understood.".into()))?;

    let what = "Failed to get authorization for Mojang services";
    let body = json!({
        "Properties": { "SandboxId": "RETAIL", "UserTokens": [account.utoken.token] },
        "RelyingParty": "rp://api.minecraftservices.com/",
        "TokenType": "JWT"
    });
    let (status, response) = send(xbox_headers(http.post(&endpoints.xsts)).json(&body), what).await?;
    if let Some(reason) = xsts_error(&response).filter(|_| !status.is_success()) {
        return Err(Error::Failed(reason));
    }
    check(status, what)?;
    account.xrp_mc = x_token(&response)
        .ok_or_else(|| Error::Failed("Could not parse authorization response for access to Mojang services.".into()))?;
    if account.xrp_mc.extra.get("uhs") != account.utoken.extra.get("uhs") {
        return Err(Error::Failed("Server has changed Mojang authorization user hash in the reply. Something is wrong.".into()));
    }

    let what = "Failed to get Minecraft access token";
    let uhs = account.xrp_mc.extra.get("uhs").and_then(Value::as_str).unwrap_or_default();
    let body = json!({ "xtoken": format!("XBL3.0 x={uhs};{}", account.xrp_mc.token), "platform": "PC_LAUNCHER" });
    let (status, response) = send(http.post(&endpoints.launcher_login).json(&body), what).await?;
    check(status, what)?;
    let (token, lifetime) = (text(&response, "access_token"), response.get("expires_in").and_then(Value::as_i64));
    let (Some(lifetime), false) = (lifetime, token.is_empty()) else {
        return Err(Error::Failed("Failed to parse the Minecraft access token response.".into()));
    };
    account.ygg = Token { iat: Some(now()), exp: Some(now() + lifetime), token, ..Token::default() };
    let bearer = format!("Bearer {}", account.ygg.token);

    let what = "Failed to determine game ownership";
    let request_id = uuid::Uuid::new_v4().to_string();
    let request = http.get(&endpoints.entitlements).query(&[("requestId", &request_id)]).header("Authorization", &bearer);
    let (status, response) = send(request, what).await?;
    // The original carries on without an answer here; ownership then stays unknown.
    account.entitlement = status.is_success().then(|| {
        let names: Vec<&str> = response
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("name")?.as_str())
            .collect();
        Entitlement { can_play_minecraft: names.contains(&"game_minecraft"), owns_minecraft: names.contains(&"product_minecraft") }
    });

    let what = "Minecraft Java profile acquisition failed";
    let (status, response) = send(http.get(&endpoints.profile).header("Authorization", &bearer), what).await?;
    if status == reqwest::StatusCode::NOT_FOUND {
        // A valid state: the account has never created a profile.
        account.profile = None;
    } else {
        check(status, what)?;
        account.profile = Some(parse_profile(&response).ok_or_else(|| Error::Failed("Minecraft Java profile response could not be parsed".into()))?);
    }
    account.state = AccountState::Online;
    Ok(())
}

fn parse_profile(body: &Value) -> Option<Profile> {
    let active = |list: &str| {
        body.get(list).and_then(Value::as_array).into_iter().flatten().find(|entry| text(entry, "state") == "ACTIVE")
    };
    let skin = match active("skins") {
        Some(skin) => {
            let url = text(skin, "url").replace("http://textures.minecraft.net", "https://textures.minecraft.net");
            json!({ "id": text(skin, "id"), "url": url, "variant": text(skin, "variant") })
        }
        None => json!({ "id": "", "url": "", "variant": "" }),
    };
    let capes = body.get("capes").and_then(Value::as_array).into_iter().flatten();
    let capes = capes.map(|cape| json!({ "alias": text(cape, "alias"), "id": text(cape, "id"), "url": text(cape, "url") }));
    Some(Profile {
        cape: active("capes").map(|cape| text(cape, "id")),
        capes: capes.collect(),
        id: Some(text(body, "id")).filter(|id| !id.is_empty())?,
        name: Some(text(body, "name")).filter(|name| !name.is_empty())?,
        skin,
    })
}

/// A full login. `show_code` gets the code the user has to enter in a browser.
pub async fn login_with_device_code(
    client: &Client,
    endpoints: &Endpoints,
    client_id: &str,
    show_code: impl FnOnce(&DeviceCode),
) -> Result<Account> {
    let code = request_device_code(client, endpoints, client_id).await?;
    show_code(&code);
    let mut account = Account::new_msa(client_id);
    account.msa = poll_device_code(client, endpoints, client_id, &code).await?;
    finish(client, endpoints, &mut account).await?;
    Ok(account)
}

/// Renews every token of a Microsoft account. The outcome is also recorded in `account.state`.
pub async fn refresh(client: &Client, endpoints: &Endpoints, client_id: &str, account: &mut Account) -> Result<()> {
    let mut updated = account.clone();
    let result = match refresh_msa_token(client, endpoints, client_id, &mut updated).await {
        Ok(()) => finish(client, endpoints, &mut updated).await,
        Err(e) => Err(e),
    };
    match &result {
        Ok(()) => *account = updated,
        Err(Error::Network(_)) => account.state = AccountState::Offline,
        Err(Error::Expired(_)) => account.state = AccountState::Expired,
        Err(e) => account.state = AccountState::Errored(e.to_string()),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    type Responder = Arc<dyn Fn(&str, usize) -> (u16, String) + Send + Sync>;

    /// A local server; `respond` gets the request path and the number of earlier requests to it.
    async fn serve(respond: Responder) -> Endpoints {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let token_calls = Arc::new(AtomicUsize::new(0));
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let (respond, token_calls) = (respond.clone(), token_calls.clone());
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 16384];
                    let read = socket.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..read]).into_owned();
                    let path = request.split_whitespace().nth(1).unwrap_or_default().split('?').next().unwrap_or_default().to_string();
                    let earlier = if path == "/token" { token_calls.fetch_add(1, Ordering::SeqCst) } else { 0 };
                    let (status, body) = respond(&path, earlier);
                    let head = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = socket.write_all(format!("{head}{body}").as_bytes()).await;
                });
            }
        });
        Endpoints {
            device_code: format!("{base}/devicecode"),
            token: format!("{base}/token"),
            xbox_user: format!("{base}/xbox"),
            xsts: format!("{base}/xsts"),
            launcher_login: format!("{base}/login"),
            entitlements: format!("{base}/entitlements"),
            profile: format!("{base}/profile"),
        }
    }

    fn happy_path(path: &str, earlier_token_calls: usize) -> (u16, String) {
        let body = match path {
            "/devicecode" => r#"{"device_code":"DEV","user_code":"ABCD-1234","verification_uri":"https://example.invalid/link","expires_in":900,"interval":1}"#,
            "/token" if earlier_token_calls == 0 => return (400, r#"{"error":"authorization_pending"}"#.into()),
            "/token" => r#"{"access_token":"MSA","refresh_token":"REFRESH","expires_in":3600,"token_type":"Bearer"}"#,
            "/xbox" => r#"{"IssueInstant":"2026-01-01T00:00:00Z","NotAfter":"2026-01-15T00:00:00Z","Token":"UTOKEN","DisplayClaims":{"xui":[{"uhs":"HASH"}]}}"#,
            "/xsts" => r#"{"Token":"XSTS","DisplayClaims":{"xui":[{"uhs":"HASH"}]}}"#,
            "/login" => r#"{"username":"u","access_token":"YGG","token_type":"Bearer","expires_in":86400}"#,
            "/entitlements" => r#"{"items":[{"name":"product_minecraft"},{"name":"game_minecraft"}]}"#,
            "/profile" => r#"{"id":"0123456789abcdef0123456789abcdef","name":"Player","skins":[{"id":"s","state":"ACTIVE","url":"http://textures.minecraft.net/texture/y","variant":"SLIM"}],"capes":[{"id":"c","state":"ACTIVE","url":"u","alias":"Migrator"}]}"#,
            _ => return (404, "{}".into()),
        };
        (200, body.to_string())
    }

    #[tokio::test]
    async fn device_code_login_fills_the_account() {
        let endpoints = serve(Arc::new(happy_path)).await;
        let client = Client::new().expect("client");
        let mut shown = None;
        let account = login_with_device_code(&client, &endpoints, "client-id", |code| shown = Some(code.user_code.clone()))
            .await
            .expect("login");

        assert_eq!(shown.as_deref(), Some("ABCD-1234"));
        assert_eq!(account.msa_client_id, "client-id");
        assert_eq!((account.msa.token.as_str(), account.msa.refresh_token.as_str()), ("MSA", "REFRESH"));
        assert_eq!((account.utoken.token.as_str(), account.xrp_mc.token.as_str(), account.ygg.token.as_str()), ("UTOKEN", "XSTS", "YGG"));
        assert!(account.owns_minecraft());
        let profile = account.profile.as_ref().expect("profile");
        assert_eq!((profile.name.as_str(), profile.cape.as_deref()), ("Player", Some("c")));
        assert_eq!(profile.skin["url"], "https://textures.minecraft.net/texture/y");
        assert_eq!(account.state, AccountState::Online);
        assert!(!account.should_refresh());
        assert_eq!(account.session(None).session, "token:YGG:0123456789abcdef0123456789abcdef");
    }

    #[tokio::test]
    async fn refresh_renews_tokens_and_reports_a_changed_client_id() {
        let endpoints = serve(Arc::new(|path, _| happy_path(path, 1))).await;
        let client = Client::new().expect("client");
        let mut account = Account::new_msa("client-id");
        account.msa.refresh_token = "OLD".into();

        refresh(&client, &endpoints, "client-id", &mut account).await.expect("refresh");
        assert_eq!((account.msa.refresh_token.as_str(), account.ygg.token.as_str()), ("REFRESH", "YGG"));

        let error = refresh(&client, &endpoints, "another-id", &mut account).await.expect_err("must fail");
        assert!(matches!(error, Error::Expired(_)));
        assert_eq!(account.state, AccountState::Expired);
        assert_eq!(account.ygg.token, "YGG", "a failed refresh keeps the old tokens");
    }

    #[tokio::test]
    async fn xsts_errors_are_explained_and_missing_profile_is_fine() {
        let no_xbox: Responder = Arc::new(|path, _| match path {
            "/xsts" => (401, r#"{"XErr":2148916233,"Message":"","Redirect":""}"#.into()),
            other => happy_path(other, 1),
        });
        let client = Client::new().expect("client");
        let mut account = Account::new_msa("client-id");
        account.msa.refresh_token = "R".into();
        let error = refresh(&client, &serve(no_xbox).await, "client-id", &mut account).await.expect_err("must fail");
        assert!(error.to_string().contains("does not have an Xbox Live profile"), "{error}");
        assert!(matches!(account.state, AccountState::Errored(_)));

        let no_profile: Responder = Arc::new(|path, _| match path {
            "/profile" => (404, r#"{"error":"NOT_FOUND"}"#.into()),
            "/entitlements" => (200, r#"{"items":[]}"#.into()),
            other => happy_path(other, 1),
        });
        refresh(&client, &serve(no_profile).await, "client-id", &mut account).await.expect("refresh");
        assert!(account.profile.is_none() && !account.owns_minecraft());
        assert_eq!(account.profile_name(), "No Minecraft profile");
    }

    #[tokio::test]
    async fn unreachable_servers_mean_offline() {
        let endpoints = Endpoints { token: "http://127.0.0.1:9/token".into(), ..Endpoints::default() };
        let mut account = Account::new_msa("client-id");
        account.msa.refresh_token = "R".into();
        let error = refresh(&Client::new().expect("client"), &endpoints, "client-id", &mut account).await.expect_err("must fail");
        assert!(matches!(error, Error::Network(_)), "{error}");
        assert_eq!(account.state, AccountState::Offline);
    }
}
