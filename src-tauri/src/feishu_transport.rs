use futures_util::{SinkExt, StreamExt};
use prost::Message as ProstMessage;
use reqwest::{Client as HttpClient, RequestBuilder, Url};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::json;
use std::{collections::HashMap, convert::Infallible, sync::Arc, time::Duration};
use tokio::{
    net::TcpStream,
    sync::{Mutex, watch},
    time::{Instant, sleep, sleep_until, timeout},
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

const API: &str = "https://open.feishu.cn";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_PAYLOAD: usize = 1024 * 1024;

pub(crate) struct Config {
    pub app_id: String,
    pub app_secret: String,
}

pub(crate) struct IncomingMessage {
    pub message_id: String,
    pub chat_id: String,
    pub sender_open_id: String,
    pub text: String,
    pub is_group: bool,
    pub mentions_bot: bool,
}

pub(crate) enum Status {
    Connecting,
    Connected,
    Disconnected(String),
    AppDeleted,
}

pub(crate) const APP_DELETED: &str = "飞书应用已被删除";

#[derive(Clone)]
pub(crate) struct Client {
    config: Arc<Config>,
    http: Result<HttpClient, &'static str>,
    token: Arc<Mutex<Option<Token>>>,
}

struct Token {
    value: String,
    refresh_at: Instant,
}

#[derive(Deserialize)]
struct ApiResponse {
    code: i64,
}

#[derive(Deserialize)]
struct ReplyResponse {
    code: i64,
    data: Option<ReplyData>,
}

#[derive(Deserialize)]
struct ReplyData {
    message_id: String,
}

#[derive(Deserialize)]
struct ReactionResponse {
    code: i64,
    data: Option<ReactionData>,
}

#[derive(Deserialize)]
struct ReactionData {
    reaction_id: String,
}

pub(crate) struct MarkdownStream {
    card_id: String,
    message_id: String,
    sequence: u64,
}

fn markdown_post(text: &str) -> serde_json::Value {
    json!({"zh_cn": {"title": "", "content": [[{"tag": "md", "text": text}]]}})
}

fn valid_message_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[derive(Deserialize)]
struct TokenResponse {
    code: i64,
    tenant_access_token: Option<String>,
    expire: Option<u64>,
}

#[derive(Deserialize)]
struct BotResponse {
    code: i64,
    bot: Option<Bot>,
}

#[derive(Deserialize)]
struct Bot {
    open_id: String,
    app_name: Option<String>,
}

#[derive(Deserialize)]
struct EndpointResponse {
    code: i64,
    data: Option<Endpoint>,
}

#[derive(Deserialize)]
struct Endpoint {
    #[serde(rename = "URL")]
    url: String,
    #[serde(rename = "ClientConfig")]
    config: Option<SocketConfig>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct SocketConfig {
    ping_interval: Option<u64>,
    reconnect_interval: Option<u64>,
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

impl Client {
    pub(crate) fn new(config: Config) -> Self {
        let http = HttpClient::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| "Could not initialize Feishu HTTPS client");
        Self {
            config: Arc::new(config),
            http,
            token: Arc::new(Mutex::new(None)),
        }
    }

    fn http(&self) -> Result<&HttpClient, String> {
        self.http.as_ref().map_err(|error| error.to_string())
    }

    pub(crate) async fn probe(&self) -> Result<(), String> {
        self.bot_open_id().await?;
        self.endpoint().await?;
        Ok(())
    }

    async fn tenant_token(&self) -> Result<String, String> {
        let mut cache = self.token.lock().await;
        if let Some(token) = cache
            .as_ref()
            .filter(|token| Instant::now() < token.refresh_at)
        {
            return Ok(token.value.clone());
        }
        let response: TokenResponse = request_json(
            self.http()?
                .post(format!(
                    "{API}/open-apis/auth/v3/tenant_access_token/internal"
                ))
                .json(&json!({
                    "app_id": self.config.app_id,
                    "app_secret": self.config.app_secret,
                })),
            "Feishu tenant token",
        )
        .await?;
        check_code(response.code, "Feishu tenant token")?;
        let value = response
            .tenant_access_token
            .filter(|token| !token.is_empty())
            .ok_or_else(|| "Feishu tenant token response was incomplete".to_owned())?;
        let ttl = response
            .expire
            .unwrap_or_default()
            .saturating_sub(60)
            .min(7200);
        *cache = Some(Token {
            value: value.clone(),
            refresh_at: Instant::now() + Duration::from_secs(ttl),
        });
        Ok(value)
    }

    pub(crate) async fn bot_name(&self) -> Option<String> {
        let name = self.bot_info().await.ok()?.app_name?;
        let name = name.trim();
        if name.is_empty() || name.chars().any(char::is_control) { return None; }
        let mut end = name.len().min(120);
        while !name.is_char_boundary(end) { end -= 1; }
        Some(name[..end].to_owned())
    }

    async fn bot_info(&self) -> Result<Bot, String> {
        let token = self.tenant_token().await?;
        let response: BotResponse = request_json(
            self.http()?
                .get(format!("{API}/open-apis/bot/v3/info"))
                .bearer_auth(token),
            "Feishu bot identity",
        )
        .await?;
        check_code(response.code, "Feishu bot identity")?;
        response.bot.filter(|bot| !bot.open_id.is_empty()).ok_or_else(|| {
            "Feishu bot identity was unavailable. Enable the bot capability for the app.".to_owned()
        })
    }

    async fn bot_open_id(&self) -> Result<String, String> {
        Ok(self.bot_info().await?.open_id)
    }

    async fn endpoint(&self) -> Result<(Url, i32, SocketConfig), String> {
        let response: EndpointResponse = request_json(
            self.http()?
                .post(format!("{API}/callback/ws/endpoint"))
                .header("locale", "zh")
                .json(&json!({
                    "AppID": self.config.app_id,
                    "AppSecret": self.config.app_secret,
                })),
            "Feishu WebSocket discovery",
        )
        .await?;
        check_code(response.code, "Feishu WebSocket discovery")?;
        let endpoint = response
            .data
            .ok_or_else(|| "Feishu WebSocket discovery returned no endpoint".to_owned())?;
        let (url, service) = validate_endpoint(&endpoint.url)?;
        Ok((url, service, endpoint.config.unwrap_or_default()))
    }

    pub(crate) async fn reply(&self, message_id: &str, text: &str) -> Result<(), String> {
        self.reply_content(message_id, "text", json!({ "text": text }))
            .await
            .map(|_| ())
    }

    pub(crate) async fn reply_markdown(
        &self,
        message_id: &str,
        text: &str,
    ) -> Result<String, String> {
        self.reply_content(message_id, "post", markdown_post(text))
            .await?
            .filter(|id| valid_message_id(id))
            .ok_or_else(|| "Feishu did not return the reply message ID".into())
    }

    pub(crate) async fn start_markdown_stream(
        &self,
        origin: &str,
    ) -> Result<MarkdownStream, String> {
        #[derive(Deserialize)]
        struct Created {
            code: i64,
            data: Option<CardData>,
        }
        #[derive(Deserialize)]
        struct CardData {
            card_id: String,
        }
        let card = json!({
            "schema": "2.0",
            "config": {
                "streaming_mode": true,
                "summary": { "content": "[生成中]" },
                "streaming_config": {
                    "print_frequency_ms": { "default": 70 },
                    "print_step": { "default": 1 },
                    "print_strategy": "fast"
                }
            },
            "body": { "elements": [{ "tag": "markdown", "element_id": "stream_md", "content": "..." }] }
        });
        let created: Created = request_json(
            self.http()?
                .post(format!("{API}/open-apis/cardkit/v1/cards"))
                .bearer_auth(self.tenant_token().await?)
                .json(&json!({ "type": "card_json", "data": card.to_string() })),
            "Feishu create Markdown stream",
        )
        .await?;
        check_code(created.code, "Feishu create Markdown stream")?;
        let card_id = created
            .data
            .map(|data| data.card_id)
            .filter(|id| valid_message_id(id))
            .ok_or("Feishu did not return a card ID")?;
        let message_id = self
            .reply_content(
                origin,
                "interactive",
                json!({ "type": "card", "data": { "card_id": card_id } }),
            )
            .await?
            .filter(|id| valid_message_id(id))
            .ok_or("Feishu did not return a message ID")?;
        Ok(MarkdownStream {
            card_id,
            message_id,
            sequence: 0,
        })
    }

    pub(crate) async fn write_markdown_stream(
        &self,
        stream: &mut MarkdownStream,
        text: &str,
    ) -> Result<(), String> {
        stream.sequence += 1;
        let MarkdownStream {
            card_id, sequence, ..
        } = stream;
        let response: ApiResponse = request_json(
            self.http()?.put(format!("{API}/open-apis/cardkit/v1/cards/{card_id}/elements/stream_md/content"))
                .bearer_auth(self.tenant_token().await?)
                .json(&json!({ "content": text, "sequence": sequence, "uuid": format!("c_{card_id}_{sequence}") })),
            "Feishu update Markdown component",
        ).await?;
        check_code(response.code, "Feishu update Markdown component")
    }

    pub(crate) async fn close_markdown_stream(
        &self,
        stream: &mut MarkdownStream,
        text: &str,
    ) -> Result<(), String> {
        stream.sequence += 1;
        let MarkdownStream {
            card_id, sequence, ..
        } = stream;
        let summary = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(60)
            .collect::<String>();
        let settings =
            json!({ "config": { "streaming_mode": false, "summary": { "content": summary } } });
        let response: ApiResponse = request_json(
            self.http()?.patch(format!("{API}/open-apis/cardkit/v1/cards/{card_id}/settings"))
                .bearer_auth(self.tenant_token().await?)
                .json(&json!({ "settings": settings.to_string(), "sequence": sequence, "uuid": format!("s_{card_id}_{sequence}") })),
            "Feishu close Markdown stream",
        ).await?;
        check_code(response.code, "Feishu close Markdown stream")
    }

    pub(crate) async fn recall_markdown_stream(&self, stream: &MarkdownStream) -> Result<(), String> {
        let response: ApiResponse = request_json(
            self.http()?
                .delete(format!(
                    "{API}/open-apis/im/v1/messages/{}",
                    stream.message_id
                ))
                .bearer_auth(self.tenant_token().await?),
            "Feishu recall Markdown stream",
        )
        .await?;
        check_code(response.code, "Feishu recall Markdown stream")
    }

    pub(crate) async fn working_reaction(&self, message_id: &str) -> Option<String> {
        if !valid_message_id(message_id) {
            return None;
        }
        let url = format!("{API}/open-apis/im/v1/messages/{message_id}/reactions");
        let response: ReactionResponse = request_json(
            self.http()
                .ok()?
                .post(url)
                .timeout(Duration::from_secs(3))
                .bearer_auth(self.tenant_token().await.ok()?)
                .json(&json!({ "reaction_type": { "emoji_type": "Typing" } })),
            "Feishu working reaction",
        )
        .await
        .ok()?;
        if response.code != 0 {
            eprintln!(
                "memdock: Feishu working reaction rejected (code {})",
                response.code
            );
            return None;
        }
        response.data.map(|data| data.reaction_id)
    }

    pub(crate) async fn remove_working_reaction(&self, message_id: &str, reaction_id: &str) {
        if !valid_message_id(message_id) || reaction_id.is_empty() || reaction_id.len() > 512 {
            return;
        }
        if let (Ok(http), Ok(token)) = (self.http(), self.tenant_token().await) {
            let Ok(mut url) = Url::parse(&format!(
                "{API}/open-apis/im/v1/messages/{message_id}/reactions"
            )) else {
                return;
            };
            if let Ok(mut segments) = url.path_segments_mut() {
                segments.push(reaction_id);
            }
            let _ = http
                .delete(url)
                .timeout(Duration::from_secs(3))
                .bearer_auth(token)
                .send()
                .await;
        }
    }

    async fn reply_content(
        &self,
        message_id: &str,
        kind: &str,
        content: serde_json::Value,
    ) -> Result<Option<String>, String> {
        if !valid_message_id(message_id) {
            return Err("Invalid Feishu message ID".into());
        }
        let body = json!({
            "msg_type": kind,
            "content": content.to_string(),
        });
        let url = format!("{API}/open-apis/im/v1/messages/{message_id}/reply");
        self.send_content(&url, body).await
    }

    async fn send_content(
        &self,
        url: &str,
        body: serde_json::Value,
    ) -> Result<Option<String>, String> {
        let token = self.tenant_token().await?;
        let mut response: ReplyResponse = request_json(
            self.http()?.post(url).bearer_auth(&token).json(&body),
            "Feishu message reply",
        )
        .await?;

        // Retry only an explicit token rejection to avoid posting duplicate replies.
        if matches!(response.code, 99991663 | 99991664 | 99991668) {
            let mut cache = self.token.lock().await;
            if cache.as_ref().is_some_and(|cached| cached.value == token) {
                *cache = None;
            }
            drop(cache);
            let fresh = self.tenant_token().await?;
            response = request_json(
                self.http()?.post(url).bearer_auth(fresh).json(&body),
                "Feishu message reply",
            )
            .await?;
        }

        check_code(response.code, "Feishu message reply")?;
        Ok(response.data.map(|data| data.message_id))
    }

    pub(crate) async fn run(
        &self,
        mut stop: watch::Receiver<bool>,
        on_message: impl Fn(IncomingMessage) -> bool,
        on_status: impl Fn(Status),
    ) {
        let reconnect = async {
            let mut failed_attempts = 0_u32;
            loop {
                on_status(Status::Connecting);
                let result = self.open_socket().await;
                let (error, delay) = match result {
                    Ok((mut socket, service, config, bot_open_id)) => {
                        failed_attempts = 0;
                        on_status(Status::Connected);
                        let delay = config.reconnect_interval.unwrap_or(5).clamp(2, 60);
                        let Err(error) = listen(
                            &mut socket,
                            service,
                            config,
                            &bot_open_id,
                            &self.config.app_id,
                            &on_message,
                        )
                        .await;
                        (error.to_owned(), delay)
                    }
                    Err(error) => {
                        let delay = (2_u64 << failed_attempts.min(5)).min(60);
                        failed_attempts = failed_attempts.saturating_add(1);
                        (error, delay)
                    }
                };
                if error == APP_DELETED {
                    on_status(Status::AppDeleted);
                    break;
                }
                on_status(Status::Disconnected(error));
                sleep(Duration::from_secs(delay)).await;
            }
        };
        tokio::select! {
            biased;
            _ = stop.wait_for(|stopped| *stopped) => {}
            _ = reconnect => {}
        }
    }

    async fn open_socket(&self) -> Result<(Socket, i32, SocketConfig, String), String> {
        let bot = self.bot_open_id().await?;
        let (url, service, config) = self.endpoint().await?;
        let websocket_config = WebSocketConfig::default()
            .max_message_size(Some(MAX_PAYLOAD))
            .max_frame_size(Some(MAX_PAYLOAD));
        let (socket, _) = timeout(
            REQUEST_TIMEOUT,
            connect_async_with_config(url.as_str(), Some(websocket_config), false),
        )
        .await
        .map_err(|_| "Feishu WebSocket handshake timed out")?
        .map_err(|error| match error {
            tokio_tungstenite::tungstenite::Error::Http(response) => {
                format!(
                    "Feishu WebSocket handshake failed (HTTP {})",
                    response.status().as_u16()
                )
            }
            _ => "Feishu WebSocket handshake failed".to_owned(),
        })?;
        Ok((socket, service, config, bot))
    }
}

async fn request_json<T: DeserializeOwned>(
    request: RequestBuilder,
    operation: &str,
) -> Result<T, String> {
    let response = request.send().await.map_err(|error| {
        let reason = if error.is_timeout() {
            "timed out"
        } else {
            "request failed"
        };
        format!("{operation}: {reason}")
    })?;
    if !response.status().is_success() {
        return Err(format!(
            "{operation} failed (HTTP {})",
            response.status().as_u16()
        ));
    }
    response
        .json()
        .await
        .map_err(|_| format!("{operation} returned an invalid response"))
}

fn check_code(code: i64, operation: &str) -> Result<(), String> {
    match code {
        0 => Ok(()),
        10217 => Err(APP_DELETED.into()),
        _ => Err(format!("{operation} failed (Feishu code {code})")),
    }
}

fn validate_endpoint(raw: &str) -> Result<(Url, i32), String> {
    let invalid = || "Feishu returned an untrusted WebSocket endpoint".to_owned();
    let url = Url::parse(raw).map_err(|_| invalid())?;
    if url.scheme() != "wss"
        || !url
            .host_str()
            .is_some_and(|host| host.ends_with(".feishu.cn"))
        || url.port().is_some_and(|port| port != 443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    let service = url
        .query_pairs()
        .find(|(key, _)| key == "service_id")
        .and_then(|(_, value)| value.parse::<i32>().ok())
        .filter(|service| *service > 0)
        .ok_or_else(invalid)?;
    Ok((url, service))
}

// Feishu's pbbp2 frame uses proto2 required fields, including zero-valued ping fields.
#[derive(Clone, PartialEq, ProstMessage)]
struct Frame {
    #[prost(uint64, required, tag = "1")]
    seq_id: u64,
    #[prost(uint64, required, tag = "2")]
    log_id: u64,
    #[prost(int32, required, tag = "3")]
    service: i32,
    #[prost(int32, required, tag = "4")]
    method: i32,
    #[prost(message, repeated, tag = "5")]
    headers: Vec<FrameHeader>,
    #[prost(string, optional, tag = "6")]
    payload_encoding: Option<String>,
    #[prost(string, optional, tag = "7")]
    payload_type: Option<String>,
    #[prost(bytes, optional, tag = "8")]
    payload: Option<Vec<u8>>,
    #[prost(string, optional, tag = "9")]
    log_id_new: Option<String>,
}

#[derive(Clone, PartialEq, ProstMessage)]
struct FrameHeader {
    #[prost(string, required, tag = "1")]
    key: String,
    #[prost(string, required, tag = "2")]
    value: String,
}

impl Frame {
    fn header(&self, key: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|header| header.key == key)
            .map(|header| header.value.as_str())
    }

    fn ping(service: i32) -> Self {
        let mut frame = Self {
            service,
            ..Self::default()
        };
        frame.headers.push(FrameHeader {
            key: "type".into(),
            value: "ping".into(),
        });
        frame
    }

    fn acknowledge(&mut self, code: u16, started: Instant) {
        self.headers.push(FrameHeader {
            key: "biz_rt".into(),
            value: started.elapsed().as_millis().to_string(),
        });
        self.payload = Some(
            json!({ "code": code, "headers": null, "data": null })
                .to_string()
                .into_bytes(),
        );
    }
}

async fn send(socket: &mut Socket, message: Message) -> Result<(), &'static str> {
    timeout(WRITE_TIMEOUT, socket.send(message))
        .await
        .map_err(|_| "Feishu WebSocket write timed out")?
        .map_err(|_| "Feishu WebSocket write failed")
}

async fn listen(
    socket: &mut Socket,
    service: i32,
    config: SocketConfig,
    bot: &str,
    app_id: &str,
    on_message: &impl Fn(IncomingMessage) -> bool,
) -> Result<Infallible, &'static str> {
    let mut interval = ping_interval(config.ping_interval.unwrap_or(120));
    let mut ping_at = Instant::now();
    let mut last_received = ping_at;
    let mut fragments = Fragments::default();
    loop {
        tokio::select! {
            _ = sleep_until(last_received + interval * 2 + Duration::from_secs(10)) => {
                return Err("Feishu WebSocket heartbeat timed out");
            }
            _ = sleep_until(ping_at) => {
                send(socket, Message::Binary(Frame::ping(service).encode_to_vec().into())).await?;
                ping_at = Instant::now() + interval;
            }
            message = socket.next() => {
                last_received = Instant::now();
                match message {
                    Some(Ok(Message::Binary(data))) => {
                        let Ok(mut frame) = Frame::decode(data) else { continue };
                        match (frame.method, frame.header("type")) {
                            (0, Some("pong")) => {
                                if let Some(new_interval) = frame.payload.as_deref()
                                    .and_then(|payload| serde_json::from_slice::<SocketConfig>(payload).ok())
                                    .and_then(|config| config.ping_interval)
                                {
                                    interval = ping_interval(new_interval);
                                    ping_at = Instant::now() + interval;
                                }
                            }
                            (1, Some("event")) => {
                                let Some(payload) = fragments.take_payload(&mut frame) else { continue };
                                // Only acknowledge successful admission. The callback must
                                // enqueue immediately so it never holds the socket open.
                                let admitted = parse_event(&payload, bot, app_id)
                                    .is_ok_and(|event| event.is_none_or(on_message));
                                frame.acknowledge(if admitted { 200 } else { 500 }, last_received);
                                send(socket, Message::Binary(frame.encode_to_vec().into())).await?;
                            }
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Ping(_))) => {
                        // Tungstenite queues the WebSocket-level pong when it reads a ping.
                        timeout(WRITE_TIMEOUT, socket.flush()).await
                            .map_err(|_| "Feishu WebSocket pong timed out")?
                            .map_err(|_| "Feishu WebSocket pong failed")?;
                    }
                    Some(Ok(Message::Close(_))) | None => return Err("Feishu WebSocket closed"),
                    Some(Err(_)) => return Err("Feishu WebSocket read failed"),
                    _ => {}
                }
            }
        }
    }
}

fn ping_interval(seconds: u64) -> Duration {
    Duration::from_secs(seconds.clamp(1, 300))
}

#[derive(Default)]
struct Fragments(HashMap<String, Batch>);

struct Batch {
    expires_at: Instant,
    parts: Vec<Option<Vec<u8>>>,
    bytes: usize,
}

impl Fragments {
    fn take_payload(&mut self, frame: &mut Frame) -> Option<Vec<u8>> {
        let sum = frame.header("sum").unwrap_or("1").parse::<usize>().ok()?;
        if sum == 1 {
            return Some(frame.payload.take().unwrap_or_default());
        }
        let seq = frame.header("seq")?.parse::<usize>().ok()?;
        if sum > 16 || seq >= sum {
            return None;
        }
        let message_id = frame
            .header("message_id")
            .filter(|id| !id.is_empty() && id.len() < 256)?
            .to_owned();
        let payload = frame.payload.take().unwrap_or_default();
        let now = Instant::now();
        self.0.retain(|_, batch| batch.expires_at > now);
        if self.0.len() >= 16 && !self.0.contains_key(&message_id) {
            return None;
        }
        let batch = self.0.entry(message_id.clone()).or_insert_with(|| Batch {
            expires_at: now + Duration::from_secs(5),
            parts: vec![None; sum],
            bytes: 0,
        });
        if batch.parts.len() != sum {
            return None;
        }
        let slot = &mut batch.parts[seq];
        if slot.is_none() {
            batch.bytes += payload.len();
            *slot = Some(payload);
        }
        if batch.bytes > MAX_PAYLOAD {
            self.0.remove(&message_id);
            return None;
        }
        if batch.parts.iter().any(Option::is_none) {
            return None;
        }
        self.0
            .remove(&message_id)
            .map(|batch| batch.parts.into_iter().flatten().flatten().collect())
    }
}

#[derive(Deserialize)]
struct EventEnvelope {
    schema: String,
    header: EventHeader,
    event: serde_json::Value,
}

#[derive(Deserialize)]
struct EventHeader {
    event_type: String,
    app_id: String,
    tenant_key: String,
}

#[derive(Deserialize)]
struct MessageEvent {
    sender: Sender,
    message: EventMessage,
}

#[derive(Deserialize)]
struct Sender {
    sender_type: String,
    sender_id: OpenId,
}

#[derive(Deserialize)]
struct OpenId {
    open_id: Option<String>,
}

#[derive(Deserialize)]
struct EventMessage {
    message_id: String,
    chat_id: String,
    chat_type: String,
    message_type: String,
    content: String,
    #[serde(default)]
    mentions: Option<Vec<Mention>>,
}

#[derive(Deserialize)]
struct Mention {
    key: String,
    id: OpenId,
}

#[derive(Deserialize)]
struct TextContent {
    text: String,
}

fn parse_event(
    payload: &[u8],
    bot: &str,
    app_id: &str,
) -> serde_json::Result<Option<IncomingMessage>> {
    let envelope: EventEnvelope = serde_json::from_slice(payload)?;
    if envelope.schema != "2.0"
        || envelope.header.app_id != app_id
        || envelope.header.event_type != "im.message.receive_v1"
    {
        return Ok(None);
    }
    let event: MessageEvent = serde_json::from_value(envelope.event)?;
    let message = event.message;
    if event.sender.sender_type != "user"
        || message.message_type != "text"
        || !matches!(message.chat_type.as_str(), "p2p" | "group")
        || message.message_id.is_empty()
        || message.chat_id.is_empty()
        || envelope.header.tenant_key.is_empty()
    {
        return Ok(None);
    }
    let Some(sender) = event
        .sender
        .sender_id
        .open_id
        .filter(|id| !id.is_empty() && id != bot)
    else {
        return Ok(None);
    };
    let content: TextContent = serde_json::from_str(&message.content)?;
    let mentions = message.mentions.unwrap_or_default();
    let bot_mention_keys: Vec<&str> = mentions
        .iter()
        .filter(|mention| mention.id.open_id.as_deref() == Some(bot))
        .map(|mention| mention.key.as_str())
        .collect();
    let header_len = content.text.find('\n').unwrap_or(content.text.len());
    let (header, body) = content.text.split_at(header_len);
    let mut text = strip_bot_mentions(header, &bot_mention_keys);
    text.push_str(body);
    Ok(Some(IncomingMessage {
        message_id: message.message_id,
        chat_id: message.chat_id,
        sender_open_id: sender,
        text,
        is_group: message.chat_type == "group",
        mentions_bot: !bot_mention_keys.is_empty(),
    }))
}

fn strip_bot_mentions(text: &str, keys: &[&str]) -> String {
    let mut remaining = text;
    let mut result = String::with_capacity(text.len());
    while let Some((before, after)) = remaining.split_once("@_user_") {
        result.push_str(before);
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        let marker_len = "@_user_".len() + digits;
        let marker = &remaining[before.len()..before.len() + marker_len];
        if !keys.contains(&marker) {
            result.push_str(marker);
        }
        remaining = &after[digits..];
    }
    result.push_str(remaining);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_encodes_required_zero_fields() {
        let bytes = Frame::ping(42).encode_to_vec();
        assert_eq!(
            bytes,
            b"\x08\x00\x10\x00\x18\x2a\x20\x00\x2a\x0c\x0a\x04type\x12\x04ping"
        );
    }

    #[test]
    fn accepts_only_feishu_wss_endpoints() {
        let valid = "wss://msg-frontier.feishu.cn/ws/v2?service_id=42&ticket=opaque";
        assert_eq!(validate_endpoint(valid).unwrap().1, 42);
        for invalid in [
            "ws://msg-frontier.feishu.cn/ws/v2?service_id=42",
            "wss://feishu.cn.example.com/ws/v2?service_id=42",
            "wss://msg-frontier.feishu.cn:444/ws/v2?service_id=42",
            "wss://user@msg-frontier.feishu.cn/ws/v2?service_id=42",
            "wss://msg-frontier.feishu.cn/ws/v2?service_id=0",
        ] {
            assert!(validate_endpoint(invalid).is_err());
        }
    }

    #[test]
    fn assembles_fragments_out_of_order_and_preserves_ack_headers() {
        fn part(seq: &str, payload: &[u8]) -> Frame {
            let headers = [("message_id", "event-id"), ("sum", "2"), ("seq", seq)]
                .into_iter()
                .map(|(key, value)| FrameHeader {
                    key: key.into(),
                    value: value.into(),
                })
                .collect();
            Frame {
                headers,
                payload: Some(payload.into()),
                seq_id: 12,
                log_id: 22,
                service: 32,
                method: 1,
                ..Frame::default()
            }
        }

        let mut fragments = Fragments::default();
        assert!(fragments.take_payload(&mut part("1", b"world")).is_none());
        let mut last = part("0", b"hello ");
        assert_eq!(
            fragments.take_payload(&mut last).as_deref(),
            Some(b"hello world".as_slice())
        );
        last.acknowledge(200, Instant::now());
        assert_eq!(last.header("message_id"), Some("event-id"));
        assert_eq!(
            (last.seq_id, last.log_id, last.service, last.method),
            (12, 22, 32, 1)
        );
        let body: serde_json::Value =
            serde_json::from_slice(last.payload.as_deref().unwrap()).unwrap();
        assert_eq!(body["code"], 200);
    }

    #[test]
    fn matches_bot_open_id_and_preserves_literal_command_body() {
        let mut payload = json!({
            "schema": "2.0",
            "header": {
                "app_id": "cli_test",
                "tenant_key": "tenant",
                "event_type": "im.message.receive_v1"
            },
            "event": {
                "sender": {
                    "sender_type": "user",
                    "sender_id": { "open_id": "ou_user" }
                },
                "message": {
                    "message_id": "om_message",
                    "chat_id": "oc_group",
                    "chat_type": "group",
                    "message_type": "text",
                    "content": json!({"text": "@_user_1 /edit @_user_10\n# @_user_1  \n"}).to_string(),
                    "mentions": [
                        { "key": "@_user_1", "id": { "open_id": "ou_bot" } },
                        { "key": "@_user_10", "id": { "open_id": "ou_other" } }
                    ]
                }
            }
        });
        let message = parse_event(&serde_json::to_vec(&payload).unwrap(), "ou_bot", "cli_test")
            .unwrap()
            .unwrap();
        assert!(message.is_group && message.mentions_bot);
        assert_eq!(message.sender_open_id, "ou_user");
        assert_eq!(message.text, " /edit @_user_10\n# @_user_1  \n");

        payload["event"]["message"]["message_type"] = json!("image");
        assert!(
            parse_event(&serde_json::to_vec(&payload).unwrap(), "ou_bot", "cli_test")
                .unwrap()
                .is_none()
        );
        payload["event"]["message"]["message_type"] = json!("text");
        payload["event"]["sender"]["sender_type"] = json!("app");
        assert!(
            parse_event(&serde_json::to_vec(&payload).unwrap(), "ou_bot", "cli_test")
                .unwrap()
                .is_none()
        );
    }
}
