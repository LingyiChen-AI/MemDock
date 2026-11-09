use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use flate2::{write::GzEncoder, Compression};
use serde::{Deserialize, Serialize};
use std::io::Write;
use tauri::{ipc::Channel, Url};
use uuid::Uuid;

// Same Feishu device registration protocol used by larksuite/cli's app_registration.go.
const ENDPOINT: &str = "https://accounts.feishu.cn/oauth/v1/app/registration";
const CREATE_PAGE: &str = "https://open.feishu.cn/page/cli";

struct Credential {
    attempt: Uuid,
    app_id: String,
    secret: String,
    expires_at: Instant,
}

#[derive(Default)]
struct RegistrationState {
    active: Option<Uuid>,
    credential: Option<Credential>,
}

static STATE: Mutex<RegistrationState> = Mutex::new(RegistrationState {
    active: None,
    credential: None,
});

#[derive(Deserialize)]
struct RegistrationReply {
    device_code: Option<String>,
    user_code: Option<String>,
    #[serde(alias = "expires_in")]
    expire_in: Option<u64>,
    interval: Option<u64>,
    error: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    user_info: Option<UserInfo>,
}

#[derive(Deserialize)]
struct UserInfo {
    tenant_brand: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredApp {
    app_id: String,
    bot_name: Option<String>,
}

fn creation_url(code: &str) -> Url {
    let mut url = Url::parse(CREATE_PAGE).expect("static Feishu URL");
    let addons = serde_json::json!({
        "preset": false,
        "scopes": { "tenant": [
            "im:message.p2p_msg:readonly",
            "im:message.group_at_msg:readonly",
            "im:message:send_as_bot",
            "im:message.reactions:write_only",
            "cardkit:card:write"
        ] },
        "events": { "items": { "tenant": ["im.message.receive_v1"] } }
    });
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(addons.to_string().as_bytes())
        .expect("gzip to memory");
    let encoded = URL_SAFE_NO_PAD.encode(encoder.finish().expect("gzip to memory"));
    url.query_pairs_mut()
        .append_pair("user_code", code)
        .append_pair("addons", &encoded);
    url
}

async fn request(
    client: &reqwest::Client,
    fields: &[(&str, &str)],
) -> Result<RegistrationReply, &'static str> {
    let mut url = Url::parse(ENDPOINT).expect("static Feishu URL");
    url.query_pairs_mut().extend_pairs(fields.iter().copied());
    let response = client
        .post(ENDPOINT)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(url.query().unwrap_or_default().to_owned())
        .send()
        .await
        .map_err(|_| "create_failed")?;
    response.json().await.map_err(|_| "create_failed")
}

async fn create(attempt: Uuid, on_url: Channel<String>) -> Result<RegisteredApp, &'static str> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "create_failed")?;
    let begin = request(
        &client,
        &[
            ("action", "begin"),
            ("archetype", "PersonalAgent"),
            ("auth_method", "client_secret"),
            ("request_user_info", "open_id tenant_brand"),
        ],
    )
    .await?;
    if begin.error.is_some() {
        return Err("create_failed");
    }
    let device_code = begin
        .device_code
        .filter(|code| !code.is_empty())
        .ok_or("create_failed")?;
    let user_code = begin
        .user_code
        .filter(|code| !code.is_empty())
        .ok_or("create_failed")?;
    let deadline = Instant::now()
        + Duration::from_secs(begin.expire_in.filter(|x| *x > 0).unwrap_or(300).min(600));
    let mut interval = begin.interval.filter(|x| *x > 0).unwrap_or(5).min(60);
    if STATE.lock().unwrap().active != Some(attempt) {
        return Err("create_cancelled");
    }
    on_url
        .send(creation_url(&user_code).to_string())
        .map_err(|_| "create_cancelled")?;

    loop {
        let next_poll = Instant::now() + Duration::from_secs(interval);
        while Instant::now() < next_poll {
            if STATE.lock().unwrap().active != Some(attempt) {
                return Err("create_cancelled");
            }
            if Instant::now() >= deadline {
                return Err("create_timeout");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        if Instant::now() >= deadline {
            return Err("create_timeout");
        }
        let Ok(reply) = request(
            &client,
            &[("action", "poll"), ("device_code", &device_code)],
        )
        .await
        else {
            interval = (interval + 1).min(60);
            continue;
        };
        if reply
            .user_info
            .as_ref()
            .and_then(|info| info.tenant_brand.as_deref())
            .is_some_and(|brand| !brand.is_empty() && brand != "feishu")
        {
            return Err("create_failed");
        }
        match reply.error.as_deref() {
            Some("authorization_pending") => continue,
            Some("slow_down") => {
                interval = (interval + 5).min(60);
                continue;
            }
            Some("expired_token" | "invalid_grant") => return Err("create_timeout"),
            Some("access_denied") => return Err("create_cancelled"),
            Some(_) => return Err("create_failed"),
            None => {}
        }
        let (Some(app_id), Some(secret)) = (reply.client_id, reply.client_secret) else {
            continue;
        };
        let valid = app_id.strip_prefix("cli_").is_some_and(|id| {
            !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        });
        if !valid || secret.trim().is_empty() {
            return Err("create_failed");
        }
        let client = crate::feishu_transport::Client::new(crate::feishu_transport::Config {
            app_id: app_id.clone(),
            app_secret: secret.clone(),
        });
        // An app waiting for approval may not expose bot info yet.
        let bot_name = tokio::time::timeout(Duration::from_secs(5), client.bot_name())
            .await
            .ok()
            .flatten();
        let mut state = STATE.lock().unwrap();
        if state.active != Some(attempt) {
            return Err("create_cancelled");
        }
        state.credential = Some(Credential {
            attempt,
            app_id: app_id.clone(),
            secret,
            expires_at: Instant::now() + Duration::from_secs(3600),
        });
        return Ok(RegisteredApp {
            app_id,
            bot_name,
        });
    }
}

#[tauri::command]
pub async fn feishu_create_app(
    attempt: String,
    on_url: Channel<String>,
) -> Result<RegisteredApp, String> {
    let id = Uuid::parse_str(&attempt).map_err(|_| "create_failed")?;
    {
        let mut state = STATE.lock().unwrap();
        if state.active.is_some() {
            return Err("create_busy".into());
        }
        state.active = Some(id);
    }
    let result = create(id, on_url).await;
    let mut state = STATE.lock().unwrap();
    if state.active == Some(id) {
        state.active = None;
    }
    result.map_err(str::to_owned)
}

pub(crate) fn registered_secret(attempt: &str, app_id: &str) -> Result<String, String> {
    let id = Uuid::parse_str(attempt).map_err(|_| "Feishu credentials are no longer available")?;
    STATE
        .lock()
        .unwrap()
        .credential
        .as_ref()
        .filter(|value| {
            value.attempt == id && value.app_id == app_id && value.expires_at > Instant::now()
        })
        .map(|value| value.secret.clone())
        .ok_or_else(|| {
            "Feishu credentials are no longer available. Enter the App Secret manually.".into()
        })
}

#[tauri::command]
pub fn feishu_cancel_create_app(attempt: String) {
    if let Ok(id) = Uuid::parse_str(&attempt) {
        let mut state = STATE.lock().unwrap();
        if state.active == Some(id) {
            state.active = None;
        }
        if state
            .credential
            .as_ref()
            .is_some_and(|value| value.attempt == id)
        {
            state.credential = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_the_user_code_without_allowing_a_redirect() {
        let url = creation_url("x&redirect=https://example.test");
        assert_eq!(url.host_str(), Some("open.feishu.cn"));
        let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(params["user_code"], "x&redirect=https://example.test");
        assert!(!params.contains_key("createOnly"));
        let compressed = URL_SAFE_NO_PAD.decode(&params["addons"]).unwrap();
        let addons: serde_json::Value =
            serde_json::from_reader(flate2::read::GzDecoder::new(compressed.as_slice())).unwrap();
        assert!(addons["scopes"]["tenant"].as_array().unwrap().iter().all(|scope| !scope.as_str().unwrap().starts_with("contact:")));
        assert_eq!(
            addons["events"]["items"]["tenant"][0],
            "im.message.receive_v1"
        );
    }
}
