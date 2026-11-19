//! Provider-neutral chat client: OpenAI-compatible and Anthropic Messages
//! adapters (text + images) with the shared guardrails:
//! - `max_tokens` (or `max_completion_tokens`) on every request
//! - prompts fitted to the model's context window
//! - never `response_format` / JSON-schema mode
//! - connect timeout 15s, total 60s + max_tokens/8
//! - response bodies capped at 4 MB, no redirects
//! - HTTPS only for cloud; http only for private hosts (checked again here)
//! - keys redacted from every error string and log line
//! - streaming (`complete_stream`, `run_stream`): the same request with
//!   `stream: true`, read as server-sent events; the caller's sink gets the
//!   answer so far each time it grows. Same timeout, same 4 MB cap. A server
//!   that answers with plain JSON, or refuses `stream`, is read the old way.

use super::config::{self, Kind};
use super::providers::{self, preset, KeyNeed, Protocol};
use once_cell::sync::Lazy;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;

pub const CHARS_PER_TOKEN: f64 = 3.2;
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
const IMAGE_TOKEN_ESTIMATE: usize = 1_600;
/// Extra room for models that think before answering (reasoning / adaptive
/// thinking count against the output cap).
const REASONING_HEADROOM: u32 = 4_096;
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    pub fn parse(s: &str) -> Role {
        match s {
            "system" => Role::System,
            "assistant" => Role::Assistant,
            _ => Role::User,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text(String),
    Image { mime: String, b64: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Msg {
    pub role: Role,
    pub parts: Vec<Part>,
}

impl Msg {
    pub fn new(role: Role, text: impl Into<String>) -> Self {
        Self { role, parts: vec![Part::Text(text.into())] }
    }
    pub fn system(text: impl Into<String>) -> Self {
        Self::new(Role::System, text)
    }
    pub fn user(text: impl Into<String>) -> Self {
        Self::new(Role::User, text)
    }
    pub fn user_with_image(text: impl Into<String>, mime: &str, b64: String) -> Self {
        Self {
            role: Role::User,
            parts: vec![Part::Image { mime: mime.to_string(), b64 }, Part::Text(text.into())],
        }
    }
    /// The message's text parts joined (images left out).
    pub fn text(&self) -> String {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn has_image(&self) -> bool {
        self.parts.iter().any(|p| matches!(p, Part::Image { .. }))
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum AiError {
    /// The user hasn't approved sending content to this cloud provider
    ConsentRequired(String),
    /// Mac App Store build: AI features need the noFriction Pro subscription
    ProRequired,
    NoProvider,
    NoKey(String),
    VisionUnavailable,
    WrongKey(String),
    NoCredit(String),
    Unreachable(String),
    BadUrl(String),
    Truncated,
    Other(String),
}

impl AiError {
    /// Stable class for the UI.
    pub fn class(&self) -> &'static str {
        match self {
            AiError::ConsentRequired(_) => "consent_required",
            AiError::ProRequired => "pro_required",
            AiError::NoProvider => "no_provider",
            AiError::NoKey(_) => "no_key",
            AiError::VisionUnavailable => "vision_unavailable",
            AiError::WrongKey(_) => "wrong_key",
            AiError::NoCredit(_) => "no_credit",
            AiError::Unreachable(_) => "unreachable",
            AiError::BadUrl(_) => "bad_url",
            AiError::Truncated => "truncated",
            AiError::Other(_) => "other",
        }
    }
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Machine-readable: the UI turns this into the consent dialog
            AiError::ConsentRequired(p) => write!(f, "CONSENT_REQUIRED:{}", p),
            // Machine-readable: the UI turns this into the paywall
            AiError::ProRequired => write!(
                f,
                "PRO_REQUIRED: AI notes, summaries, chat and briefings are part of noFriction Pro."
            ),
            AiError::NoProvider => write!(
                f,
                "AI_NO_PROVIDER: No AI provider is set up. Open Settings → AI Engine and configure your endpoint and model, or use Apple on-device."
            ),
            AiError::NoKey(p) => write!(f, "AI_NO_KEY: No API key saved for {}. Add one in Settings → AI Engine.", p),
            AiError::VisionUnavailable => write!(
                f,
                "AI_NO_VISION: The selected AI model can't read images. Pick a vision model in Settings → AI Engine."
            ),
            AiError::WrongKey(m) => write!(f, "AI_WRONG_KEY: The provider rejected the API key ({})", m),
            AiError::NoCredit(m) => write!(f, "AI_NO_CREDIT: Out of credit or rate-limited ({})", m),
            AiError::Unreachable(m) => write!(f, "AI_UNREACHABLE: Can't reach the AI provider ({})", m),
            AiError::BadUrl(m) => write!(f, "AI_BAD_URL: {}", m),
            AiError::Truncated => write!(
                f,
                "AI_TRUNCATED: The model used its whole token budget before answering. Try a non-reasoning model."
            ),
            AiError::Other(m) => write!(f, "AI_ERROR: {}", m),
        }
    }
}

impl From<AiError> for String {
    fn from(e: AiError) -> String {
        e.to_string()
    }
}

// ---------------------------------------------------------------------------
// Target resolution
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Target {
    pub provider: String,
    /// Preset id matching the URL (for per-service quirks), else `provider`
    pub hint: &'static str,
    pub protocol: Protocol,
    pub base_url: String,
    pub key: Option<String>,
    pub model: String,
    pub context_tokens: usize,
    pub completion_tokens: bool,
}

/// OpenAI reasoning families reject `max_tokens` outright; skip the failed
/// round trip for them.
pub fn prefers_completion_tokens(provider: &str, model: &str) -> bool {
    if provider != "openai" {
        return false;
    }
    let m = model.to_ascii_lowercase();
    ["o1", "o3", "o4", "gpt-5", "gpt-6"].iter().any(|p| m.starts_with(p)) && !m.contains("chat")
}

/// Resolve the active provider for `kind`, enforcing consent and URL policy.
pub fn resolve(kind: Kind) -> Result<Target, AiError> {
    let cfg = config::snapshot();
    let sel = cfg.selection(kind).ok_or(match kind {
        Kind::Text => AiError::NoProvider,
        Kind::Vision => {
            if cfg.text.is_none() {
                AiError::NoProvider
            } else {
                AiError::VisionUnavailable
            }
        }
    })?;
    resolve_for(&cfg, &sel.provider, &sel.model)
}

pub fn resolve_for(cfg: &config::AiConfig, provider: &str, model: &str) -> Result<Target, AiError> {
    resolve_inner(cfg, provider, model, true)
}

/// Resolution for the connection test only: same URL and key rules, but no
/// consent gate, because the test sends a fixed one-word prompt and never
/// any meeting content. Only `commands::ai_test` (an explicit click) uses it.
pub fn resolve_for_probe(cfg: &config::AiConfig, provider: &str, model: &str) -> Result<Target, AiError> {
    resolve_inner(cfg, provider, model, false)
}

fn resolve_inner(cfg: &config::AiConfig, provider: &str, model: &str, enforce_consent: bool) -> Result<Target, AiError> {
    let p = preset(provider).ok_or_else(|| AiError::Other(format!("Unknown provider '{}'", provider)))?;
    if p.protocol == Protocol::Apple {
        // On-device: no URL, no key, no consent (nothing leaves the Mac)
        let (ok, reason) = crate::store::apple_model_availability();
        if !ok {
            return Err(AiError::Unreachable(format!("Apple on-device model unavailable: {}", reason)));
        }
        return Ok(Target {
            provider: provider.to_string(),
            hint: providers::APPLE_PROVIDER,
            protocol: Protocol::Apple,
            base_url: providers::APPLE_BASE_URL.to_string(),
            key: None,
            model: providers::APPLE_MODEL.to_string(),
            context_tokens: providers::APPLE_CONTEXT_TOKENS,
            completion_tokens: false,
        });
    }
    let base = cfg
        .base_url(provider)
        .ok_or_else(|| AiError::BadUrl(format!("No endpoint URL set for {}", p.name)))?;
    let base = providers::check_base_url(&base).map_err(AiError::BadUrl)?;
    if enforce_consent && !cfg.is_local(provider) && !cfg.provider(provider).consent {
        super::emit_consent_required(provider);
        return Err(AiError::ConsentRequired(provider.to_string()));
    }
    let key = config::api_key(cfg, provider);
    if p.key == KeyNeed::Required && key.is_none() {
        return Err(AiError::NoKey(p.name.to_string()));
    }
    let st = cfg.provider(provider);
    let info = cfg.model_info(provider, model);
    let hint = providers::provider_hint(provider, Some(&base));
    Ok(Target {
        provider: provider.to_string(),
        hint,
        protocol: p.protocol,
        base_url: base,
        key,
        model: model.to_string(),
        context_tokens: providers::context_window(hint, model, info.and_then(|i| i.context)),
        completion_tokens: st.completion_tokens_models.iter().any(|m| m == model)
            || prefers_completion_tokens(hint, model),
    })
}

// ---------------------------------------------------------------------------
// Context fitting
// ---------------------------------------------------------------------------

pub const TRIM_MARKER: &str = "\n\n[… middle trimmed to fit the model's context window …]\n\n";

/// Trim the longest text part (keeping its head and tail) so prompt +
/// answer fit the context window. Returns true if anything was cut.
pub fn fit_messages(msgs: &mut [Msg], context_tokens: usize, max_tokens: u32) -> bool {
    let images: usize = msgs
        .iter()
        .map(|m| m.parts.iter().filter(|p| matches!(p, Part::Image { .. })).count())
        .sum();
    let avail = context_tokens
        .saturating_sub(max_tokens as usize + 512)
        .saturating_sub(images * IMAGE_TOKEN_ESTIMATE)
        .max(1_024);
    let budget = (avail as f64 * CHARS_PER_TOKEN) as usize;
    let len = |p: &Part| match p {
        Part::Text(t) => t.chars().count(),
        _ => 0,
    };
    let total: usize = msgs.iter().flat_map(|m| m.parts.iter()).map(len).sum();
    if total <= budget {
        return false;
    }
    let mut longest: Option<(usize, usize, usize)> = None;
    for (mi, m) in msgs.iter().enumerate() {
        for (pi, p) in m.parts.iter().enumerate() {
            let l = len(p);
            if longest.map_or(true, |(_, _, best)| l > best) {
                longest = Some((mi, pi, l));
            }
        }
    }
    let Some((mi, pi, l)) = longest else { return false };
    let marker_len = TRIM_MARKER.chars().count();
    let keep = l.saturating_sub(total - budget + marker_len);
    if let Part::Text(t) = &mut msgs[mi].parts[pi] {
        let chars: Vec<char> = t.chars().collect();
        let head = keep * 6 / 10;
        let tail = keep - head;
        let mut out: String = chars[..head].iter().collect();
        out.push_str(TRIM_MARKER);
        out.extend(chars[chars.len() - tail..].iter());
        *t = out;
    }
    log::warn!("AI prompt trimmed from {} to ~{} chars to fit a {}-token window", total, budget, context_tokens);
    true
}

// ---------------------------------------------------------------------------
// Request building (pure; unit tested)
// ---------------------------------------------------------------------------

pub fn request_headers(protocol: Protocol, provider: &str, key: Option<&str>) -> Vec<(&'static str, String)> {
    let mut h = vec![("content-type", "application/json".to_string())];
    match protocol {
        Protocol::OpenAI => {
            if let Some(k) = key.filter(|k| !k.is_empty()) {
                h.push(("authorization", format!("Bearer {}", k)));
            }
            if provider == "openrouter" {
                h.push(("x-title", "noFriction".to_string()));
            }
        }
        Protocol::Anthropic => {
            if let Some(k) = key.filter(|k| !k.is_empty()) {
                h.push(("x-api-key", k.to_string()));
            }
            h.push(("anthropic-version", ANTHROPIC_VERSION.to_string()));
        }
        Protocol::Apple => {}
    }
    h
}

/// OpenAI-compatible `/chat/completions` body. Never includes
/// `response_format`. `completion_tokens` switches to
/// `max_completion_tokens` and drops `temperature`.
pub fn build_openai_request(
    model: &str,
    msgs: &[Msg],
    max_tokens: u32,
    temperature: Option<f32>,
    completion_tokens: bool,
) -> Value {
    let messages: Vec<Value> = msgs
        .iter()
        .map(|m| {
            if m.has_image() {
                let parts: Vec<Value> = m
                    .parts
                    .iter()
                    .map(|p| match p {
                        Part::Text(t) => json!({"type": "text", "text": t}),
                        Part::Image { mime, b64 } => json!({
                            "type": "image_url",
                            "image_url": {"url": format!("data:{};base64,{}", mime, b64)}
                        }),
                    })
                    .collect();
                json!({"role": m.role.as_str(), "content": parts})
            } else {
                json!({"role": m.role.as_str(), "content": m.text()})
            }
        })
        .collect();
    let mut body = json!({"model": model, "messages": messages, "stream": false});
    if completion_tokens {
        body["max_completion_tokens"] = json!(max_tokens + REASONING_HEADROOM);
    } else {
        body["max_tokens"] = json!(max_tokens);
        if let Some(t) = temperature {
            body["temperature"] = json!(t);
        }
    }
    body
}

/// Newer Claude models reject sampling parameters; only send temperature to
/// the generations known to accept it.
pub fn anthropic_accepts_temperature(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    ["claude-3", "-4-5", "-4-6", "-4-1", "-4-0", "-4-2025"].iter().any(|p| m.contains(p))
}

/// Models with thinking on by default spend output tokens thinking first.
pub fn anthropic_thinks_by_default(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    ["claude-opus-5", "claude-sonnet-5", "claude-fable", "claude-mythos"].iter().any(|p| m.starts_with(p))
}

/// Anthropic Messages body: system prompts go in the top-level `system`
/// field, turns alternate user/assistant starting with user, images are
/// base64 `image` blocks, `max_tokens` is always set.
pub fn build_anthropic_request(model: &str, msgs: &[Msg], max_tokens: u32, temperature: Option<f32>) -> Value {
    let system: Vec<String> = msgs.iter().filter(|m| m.role == Role::System).map(|m| m.text()).collect();
    let mut turns: Vec<(Role, Vec<Value>)> = Vec::new();
    for m in msgs.iter().filter(|m| m.role != Role::System) {
        let blocks: Vec<Value> = m
            .parts
            .iter()
            .filter_map(|p| match p {
                Part::Text(t) if t.is_empty() => None,
                Part::Text(t) => Some(json!({"type": "text", "text": t})),
                Part::Image { mime, b64 } => Some(json!({
                    "type": "image",
                    "source": {"type": "base64", "media_type": mime, "data": b64}
                })),
            })
            .collect();
        if blocks.is_empty() {
            continue;
        }
        match turns.last_mut() {
            Some((role, content)) if *role == m.role => content.extend(blocks),
            _ => turns.push((m.role, blocks)),
        }
    }
    if turns.first().map(|(r, _)| *r) != Some(Role::User) {
        turns.insert(0, (Role::User, vec![json!({"type": "text", "text": "(context follows)"})]));
    }
    let messages: Vec<Value> = turns
        .into_iter()
        .map(|(r, c)| json!({"role": r.as_str(), "content": c}))
        .collect();
    let cap = if anthropic_thinks_by_default(model) { max_tokens + REASONING_HEADROOM } else { max_tokens };
    let mut body = json!({"model": model, "max_tokens": cap, "messages": messages});
    if !system.is_empty() {
        body["system"] = json!(system.join("\n\n"));
    }
    if let (Some(t), true) = (temperature, anthropic_accepts_temperature(model)) {
        body["temperature"] = json!(t);
    }
    body
}

/// On a 400 that names the token/temperature parameter, retry once with
/// `max_completion_tokens` and no temperature.
pub fn needs_completion_tokens_retry(status: u16, body: &str) -> bool {
    if status != 400 {
        return false;
    }
    let b = body.to_ascii_lowercase();
    b.contains("max_tokens")
        || b.contains("max_completion_tokens")
        || (b.contains("temperature") && (b.contains("unsupported") || b.contains("not support") || b.contains("default")))
}

/// Anthropic 400 caused by a sampling parameter: retry without temperature.
pub fn needs_anthropic_param_retry(status: u16, body: &str) -> bool {
    status == 400 && body.to_ascii_lowercase().contains("temperature")
}

/// Remove `<think>…</think>` blocks some local reasoning models emit.
pub fn strip_think(s: &str) -> String {
    let mut out = s.to_string();
    while let (Some(a), Some(b)) = (out.find("<think>"), out.find("</think>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</think>".len(), "");
    }
    out.trim().to_string()
}

/// The part of an answer still being written that can be shown: whole
/// `<think>` blocks are gone, a block still open hides everything after it,
/// and so does the start of a tag that hasn't finished arriving (`<thi`).
pub fn visible_so_far(raw: &str) -> String {
    let mut out = raw.to_string();
    while let (Some(a), Some(b)) = (out.find("<think>"), out.find("</think>")) {
        if b < a {
            break;
        }
        out.replace_range(a..b + "</think>".len(), "");
    }
    if let Some(a) = out.find("<think>") {
        out.truncate(a);
    } else if let Some(a) = out.rfind('<') {
        let tail = &out[a..];
        if "<think>".starts_with(tail) {
            out.truncate(a);
        }
    }
    out.trim().to_string()
}

pub fn parse_openai_response(v: &Value) -> Result<String, AiError> {
    let choice = &v["choices"][0];
    if choice.is_null() {
        return Err(AiError::Other("The provider returned no choices".into()));
    }
    let content = &choice["message"]["content"];
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    };
    let text = strip_think(&text);
    if text.is_empty() {
        if choice["finish_reason"] == "length" {
            return Err(AiError::Truncated);
        }
        return Err(AiError::Other("The model returned an empty answer".into()));
    }
    if choice["finish_reason"] == "length" {
        log::warn!("AI answer hit the token cap and was cut off");
    }
    Ok(text)
}

pub fn parse_anthropic_response(v: &Value) -> Result<String, AiError> {
    if v["stop_reason"] == "refusal" {
        return Err(AiError::Other("The model declined this request".into()));
    }
    let text: String = v["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    let text = text.trim().to_string();
    if text.is_empty() {
        if v["stop_reason"] == "max_tokens" {
            return Err(AiError::Truncated);
        }
        return Err(AiError::Other("The model returned an empty answer".into()));
    }
    Ok(text)
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

/// The shared client (no redirects, connect timeout), for the sign-in's key
/// exchange in `ai::connect`.
pub(crate) fn http() -> &'static reqwest::Client {
    &HTTP
}

static HTTP: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        // Never follow redirects: a redirect could downgrade to http or send
        // the key somewhere else.
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("noFriction-Meetings/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("HTTP client")
});

pub fn request_timeout(max_tokens: u32) -> Duration {
    Duration::from_secs(60 + max_tokens as u64 / 8)
}

fn error_message(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    if let Ok(v) = serde_json::from_str::<Value>(&text) {
        for ptr in ["/error/message", "/error", "/message", "/detail", "/error/type"] {
            if let Some(s) = v.pointer(ptr).and_then(|x| x.as_str()) {
                return s.chars().take(300).collect();
            }
        }
    }
    text.chars().take(200).collect::<String>().trim().to_string()
}

fn status_error(status: u16, body: &[u8], key: Option<&str>) -> AiError {
    let msg = providers::redact_with(&error_message(body), key);
    match status {
        401 | 403 => AiError::WrongKey(format!("{} {}", status, msg)),
        402 | 429 => AiError::NoCredit(format!("{} {}", status, msg)),
        300..=399 => AiError::Other(format!("{} redirect refused; check the base URL", status)),
        _ => AiError::Other(format!("{} {}", status, msg)),
    }
}

fn net_error(e: &reqwest::Error, key: Option<&str>) -> AiError {
    if e.is_timeout() {
        return AiError::Unreachable("timed out".into());
    }
    let mut msg = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        msg = format!("{}: {}", msg, s);
        src = s.source();
    }
    AiError::Unreachable(providers::redact_with(&msg, key))
}

async fn read_capped(mut resp: reqwest::Response, key: Option<&str>) -> Result<Vec<u8>, AiError> {
    if resp.content_length().map_or(false, |l| l as usize > MAX_BODY_BYTES) {
        return Err(AiError::Other("Response larger than 4 MB refused".into()));
    }
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| net_error(&e, key))? {
        if buf.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(AiError::Other("Response larger than 4 MB refused".into()));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

async fn send(
    req: reqwest::RequestBuilder,
    headers: &[(&'static str, String)],
    key: Option<&str>,
) -> Result<(u16, Vec<u8>), AiError> {
    let mut req = req;
    for (k, v) in headers {
        req = req.header(*k, v);
    }
    let resp = req.send().await.map_err(|e| net_error(&e, key))?;
    let status = resp.status().as_u16();
    let body = read_capped(resp, key).await?;
    Ok((status, body))
}

fn parse_json(body: &[u8]) -> Result<Value, AiError> {
    serde_json::from_slice(body).map_err(|_| AiError::Other("The provider sent a response we couldn't read".into()))
}

// ---------------------------------------------------------------------------
// Streaming (server-sent events)
// ---------------------------------------------------------------------------

/// Where a streamed answer goes while it is written: called with the whole
/// answer so far (never a fragment), each time it grows. A caller can show
/// it as is and drop any call it has no time for.
pub type Sink<'a> = &'a (dyn Fn(&str) + Send + Sync);

/// The text of a streamed answer, fed the body in whatever pieces the
/// network delivers. Pure, so the two wire formats are unit tested:
/// OpenAI-compatible `chat.completion.chunk`s and Anthropic Messages events.
/// Reasoning and thinking deltas are not part of the answer and are skipped.
pub struct SseText {
    protocol: Protocol,
    buf: Vec<u8>,
    /// The answer so far, as sent (think blocks still in it)
    pub text: String,
    /// `finish_reason` / `stop_reason`, once the server gave one
    pub finish: Option<String>,
    /// An error event inside a 200 stream: its words, and the HTTP status
    /// the same refusal would have had (401 a key, 429 credit or rate)
    pub error: Option<(u16, String)>,
    /// The server said the stream is complete
    pub done: bool,
}

impl SseText {
    pub fn new(protocol: Protocol) -> Self {
        Self { protocol, buf: Vec::new(), text: String::new(), finish: None, error: None, done: false }
    }

    /// Take the next bytes. True when the answer grew.
    pub fn feed(&mut self, bytes: &[u8]) -> bool {
        self.buf.extend_from_slice(bytes);
        let before = self.text.len();
        while let Some(nl) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=nl).collect();
            self.line(&line);
        }
        self.text.len() > before
    }

    /// The body ended: a last line may have come without its newline.
    pub fn end(&mut self) {
        let line = std::mem::take(&mut self.buf);
        self.line(&line);
    }

    fn line(&mut self, line: &[u8]) {
        let line = String::from_utf8_lossy(line);
        // `event:` names, `:` keep-alive comments and blank lines carry no text
        let Some(data) = line.trim_end_matches(['\r', '\n']).strip_prefix("data:") else { return };
        let data = data.trim();
        if data == "[DONE]" {
            self.done = true;
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else { return };
        match self.protocol {
            Protocol::Anthropic => self.anthropic(&v),
            _ => self.openai(&v),
        }
    }

    fn fail(&mut self, v: &Value) {
        let msg = ["/error/message", "/error", "/message", "/error/type"]
            .iter()
            .find_map(|p| v.pointer(p).and_then(|x| x.as_str()))
            .unwrap_or("The provider reported an error");
        // The stream had already said 200: the error's own type or code
        // says what kind it is
        let kind = ["/error/type", "/error/code"]
            .iter()
            .filter_map(|p| v.pointer(p).and_then(|x| x.as_str()))
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();
        let status = if kind.contains("authentication") || kind.contains("invalid_api_key") || kind.contains("permission") {
            401
        } else if kind.contains("rate_limit") || kind.contains("quota") || kind.contains("credit") || kind.contains("billing") {
            429
        } else {
            500
        };
        self.error = Some((status, msg.chars().take(300).collect()));
    }

    fn openai(&mut self, v: &Value) {
        if !v["error"].is_null() {
            return self.fail(v);
        }
        let choice = &v["choices"][0];
        match &choice["delta"]["content"] {
            Value::String(s) => self.text.push_str(s),
            Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).for_each(|t| self.text.push_str(t)),
            _ => {}
        }
        if let Some(f) = choice["finish_reason"].as_str() {
            self.finish = Some(f.to_string());
        }
    }

    fn anthropic(&mut self, v: &Value) {
        match v["type"].as_str().unwrap_or("") {
            "content_block_delta" if v["delta"]["type"] == "text_delta" => {
                self.text.push_str(v["delta"]["text"].as_str().unwrap_or(""));
            }
            "message_delta" => {
                if let Some(r) = v["delta"]["stop_reason"].as_str() {
                    self.finish = Some(r.to_string());
                }
            }
            "message_stop" => self.done = true,
            "error" => self.fail(v),
            _ => {}
        }
    }

    /// The finished answer, by the same rules as a whole-body response. A
    /// body that just stopped (no finish reason, no end marker) is a broken
    /// connection, not an answer: half of one is never returned as whole.
    pub fn finish(self, key: Option<&str>) -> Result<String, AiError> {
        if let Some((status, e)) = &self.error {
            let msg = providers::redact_with(e, key);
            return Err(match status {
                401 => AiError::WrongKey(msg),
                429 => AiError::NoCredit(msg),
                _ => AiError::Other(msg),
            });
        }
        if !self.done && self.finish.is_none() {
            return Err(AiError::Unreachable("the connection closed before the answer was finished".into()));
        }
        let finish = self.finish.as_deref().unwrap_or("");
        if finish == "refusal" {
            return Err(AiError::Other("The model declined this request".into()));
        }
        let cut = finish == "length" || finish == "max_tokens";
        let text = strip_think(&self.text);
        if text.is_empty() {
            return Err(if cut { AiError::Truncated } else { AiError::Other("The model returned an empty answer".into()) });
        }
        if cut {
            log::warn!("AI answer hit the token cap and was cut off");
        }
        Ok(text)
    }
}

/// What came back from a request that asked for a stream.
enum Streamed {
    /// The answer, already given to the sink piece by piece
    Text(String),
    /// Not a stream (an error status, or a server that answered with plain
    /// JSON): the status and the whole body, for the usual handling
    Whole(u16, Vec<u8>),
}

/// Send a request that asks for a stream and read it. A body that starts
/// with `{` is plain JSON whatever the request said (every SSE line starts
/// with a field name or `:`), so it is collected and handed back whole.
async fn send_streaming(
    req: reqwest::RequestBuilder,
    headers: &[(&'static str, String)],
    key: Option<&str>,
    protocol: Protocol,
    sink: Sink<'_>,
) -> Result<Streamed, AiError> {
    let mut req = req;
    for (k, v) in headers {
        req = req.header(*k, v);
    }
    let mut resp = req.send().await.map_err(|e| net_error(&e, key))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Ok(Streamed::Whole(status, read_capped(resp, key).await?));
    }
    let mut sse = SseText::new(protocol);
    let mut whole: Option<Vec<u8>> = None;
    let mut sniffed = false;
    let mut total = 0usize;
    let mut shown = String::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| net_error(&e, key))? {
        total += chunk.len();
        if total > MAX_BODY_BYTES {
            return Err(AiError::Other("Response larger than 4 MB refused".into()));
        }
        if !sniffed {
            let Some(first) = chunk.iter().find(|b| !b.is_ascii_whitespace()) else { continue };
            sniffed = true;
            if *first == b'{' {
                whole = Some(Vec::new());
            }
        }
        if let Some(buf) = whole.as_mut() {
            buf.extend_from_slice(&chunk);
            continue;
        }
        if sse.feed(&chunk) {
            let now = visible_so_far(&sse.text);
            if now != shown {
                sink(&now);
                shown = now;
            }
        }
        if sse.done || sse.error.is_some() {
            break;
        }
    }
    if let Some(buf) = whole {
        return Ok(Streamed::Whole(status, buf));
    }
    sse.end();
    let text = sse.finish(key)?;
    if text != shown {
        sink(&text);
    }
    Ok(Streamed::Text(text))
}

/// A 400 that names `stream`: this server can't stream. Ask again the plain way.
pub fn stream_refused(status: u16, body: &str) -> bool {
    status == 400 && body.to_ascii_lowercase().contains("stream")
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct Opts {
    pub max_tokens: u32,
    pub temperature: Option<f32>,
}

impl Default for Opts {
    fn default() -> Self {
        Self { max_tokens: 1_500, temperature: Some(0.3) }
    }
}

/// One chat completion on the active provider for `kind`. Every LLM call in
/// the app goes through here, so this is where the Mac App Store build checks
/// the noFriction Pro subscription (compiled out of the DMG build).
pub async fn complete(kind: Kind, msgs: Vec<Msg>, opts: Opts) -> Result<String, AiError> {
    crate::entitlement::require_pro().await?;
    let target = resolve(kind)?;
    run(&target, msgs, opts).await
}

/// `complete`, with the answer handed to `sink` while it is written (the
/// whole text so far, each time it grows). Same gate, same target rules,
/// same result. For answers a person is waiting to read (Chat, Ask).
pub async fn complete_stream(kind: Kind, msgs: Vec<Msg>, opts: Opts, sink: Sink<'_>) -> Result<String, AiError> {
    crate::entitlement::require_pro().await?;
    let target = resolve(kind)?;
    run_stream(&target, msgs, opts, sink).await
}

/// One completion that goes only where the caller was told it would. The
/// target is resolved once: `allowed` sees that target's base URL and model
/// before anything is sent, and the request then uses the same target, so
/// the check and the send can never see two different settings.
/// `Ok(None)`: `allowed` said no and nothing was sent.
///
/// Read the screens uses it: a picture goes to the host and model its
/// confirmation named, or not at all (screen_reading.rs).
pub async fn complete_if(
    kind: Kind,
    msgs: Vec<Msg>,
    opts: Opts,
    allowed: impl FnOnce(&str, &str) -> bool,
) -> Result<Option<String>, AiError> {
    crate::entitlement::require_pro().await?;
    let target = resolve(kind)?;
    if !allowed(&target.base_url, &target.model) {
        return Ok(None);
    }
    run(&target, msgs, opts).await.map(Some)
}

/// Apple Foundation Models: system messages become the session
/// instructions; the rest is flattened into one prompt (the on-device model
/// has a 4K context, already fitted by `run`).
async fn run_apple(msgs: &[Msg], max_tokens: u32, temperature: Option<f32>, sink: Option<Sink<'_>>) -> Result<String, AiError> {
    if msgs.iter().any(|m| m.has_image()) {
        return Err(AiError::VisionUnavailable);
    }
    let instructions = msgs
        .iter()
        .filter(|m| m.role == Role::System)
        .map(|m| m.text())
        .collect::<Vec<_>>()
        .join("\n\n");
    let convo: Vec<&Msg> = msgs.iter().filter(|m| m.role != Role::System).collect();
    let prompt = if convo.len() == 1 {
        convo[0].text()
    } else {
        convo
            .iter()
            .map(|m| format!("{}: {}", if m.role == Role::Assistant { "Assistant" } else { "User" }, m.text()))
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    let out = match sink {
        Some(sink) => {
            crate::store::apple_generate_stream(&instructions, &prompt, max_tokens, temperature, &|so_far| {
                sink(&visible_so_far(so_far))
            })
            .await
        }
        None => crate::store::apple_generate(&instructions, &prompt, max_tokens, temperature).await,
    };
    out.map_err(|e| AiError::Other(format!("Apple on-device model: {}", e)))
}

/// One chat completion against a resolved target.
pub async fn run(t: &Target, msgs: Vec<Msg>, opts: Opts) -> Result<String, AiError> {
    run_with(t, msgs, opts, None).await
}

/// `run`, with the answer handed to `sink` while it is written.
pub async fn run_stream(t: &Target, msgs: Vec<Msg>, opts: Opts, sink: Sink<'_>) -> Result<String, AiError> {
    run_with(t, msgs, opts, Some(sink)).await
}

/// At most this many requests for one completion: the first, and one more
/// for each thing a server can refuse (the token parameter or temperature,
/// and streaming).
const MAX_TRIES: usize = 3;

async fn run_with(t: &Target, mut msgs: Vec<Msg>, opts: Opts, sink: Option<Sink<'_>>) -> Result<String, AiError> {
    let max_tokens = opts.max_tokens.clamp(16, 16_000);
    fit_messages(&mut msgs, t.context_tokens, max_tokens);
    let key = t.key.as_deref();
    if t.protocol == Protocol::Apple {
        return run_apple(&msgs, max_tokens, opts.temperature, sink).await;
    }
    let anthropic = t.protocol == Protocol::Anthropic;
    let url = format!("{}/{}", t.base_url, if anthropic { "messages" } else { "chat/completions" });
    let headers = request_headers(t.protocol, &t.provider, key);
    let mut completion_tokens = t.completion_tokens;
    let mut temperature = opts.temperature;
    let mut streaming = sink.is_some();
    // Claude 5 via Anthropic's OpenAI-compatible layer thinks by default, and
    // thinking counts against max_tokens.
    let max_tokens = if !anthropic && t.hint == "anthropic" && anthropic_thinks_by_default(&t.model) {
        max_tokens + REASONING_HEADROOM
    } else {
        max_tokens
    };
    for _ in 0..MAX_TRIES {
        let (mut body, cap) = if anthropic {
            let body = build_anthropic_request(&t.model, &msgs, max_tokens, temperature);
            let cap = body["max_tokens"].as_u64().unwrap_or(max_tokens as u64) as u32;
            (body, cap)
        } else {
            let body = build_openai_request(&t.model, &msgs, max_tokens, temperature, completion_tokens);
            (body, if completion_tokens { max_tokens + REASONING_HEADROOM } else { max_tokens })
        };
        if streaming {
            body["stream"] = json!(true);
        }
        let req = HTTP.post(&url).json(&body).timeout(request_timeout(cap));
        let (status, bytes) = match sink.filter(|_| streaming) {
            Some(sink) => match send_streaming(req, &headers, key, t.protocol, sink).await? {
                Streamed::Text(text) => return Ok(text),
                Streamed::Whole(status, bytes) => (status, bytes),
            },
            None => send(req, &headers, key).await?,
        };
        if (200..300).contains(&status) {
            let v = parse_json(&bytes)?;
            let text = if anthropic { parse_anthropic_response(&v) } else { parse_openai_response(&v) }?;
            // Asked to stream, answered whole: the sink still gets the answer
            if let Some(sink) = sink {
                sink(&text);
            }
            return Ok(text);
        }
        let text = String::from_utf8_lossy(&bytes);
        if !anthropic && !completion_tokens && needs_completion_tokens_retry(status, &text) {
            log::info!("AI: {} wants max_completion_tokens; retrying and remembering", t.model);
            completion_tokens = true;
            let (p, m) = (t.provider.clone(), t.model.clone());
            config::update_background(move |c| {
                let st = c.provider_mut(&p);
                if !st.completion_tokens_models.contains(&m) {
                    st.completion_tokens_models.push(m);
                }
            });
            continue;
        }
        if anthropic && temperature.is_some() && needs_anthropic_param_retry(status, &text) {
            temperature = None;
            continue;
        }
        if streaming && stream_refused(status, &text) {
            log::info!("AI: {} doesn't stream; asking for the whole answer", t.provider);
            streaming = false;
            continue;
        }
        let err = status_error(status, &bytes, key);
        log::warn!("AI request to {} failed: {}", t.provider, err);
        return Err(err);
    }
    Err(AiError::Other("Request rejected twice".into()))
}

// ---------------------------------------------------------------------------
// Connection test (explicit click; one fixed word, never meeting content)
// ---------------------------------------------------------------------------

/// Plain-words result of the connection test for the settings screen.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProbeOutcome {
    pub ok: bool,
    /// connected | wrong_key | no_credit | unreachable | bad_url | model_missing | no_key | other
    pub class: &'static str,
    pub message: String,
}

impl ProbeOutcome {
    fn new(ok: bool, class: &'static str, message: String) -> Self {
        Self { ok, class, message }
    }

    /// Map a resolution error (no key, bad URL, …) to a test outcome.
    pub fn from_error(e: &AiError, host: &str) -> Self {
        let text = e.to_string();
        let detail = text.splitn(2, ": ").nth(1).unwrap_or(&text).to_string();
        match e {
            AiError::NoKey(_) => Self::new(false, "no_key", "No API key is saved for this endpoint. Paste your key and save first.".into()),
            AiError::BadUrl(m) => Self::new(false, "bad_url", format!("Check the base URL: {}", m)),
            AiError::Unreachable(m) => Self::new(false, "unreachable", format!("Couldn't reach {}: {}", host, m)),
            AiError::WrongKey(_) => Self::new(false, "wrong_key", format!("{} rejected the API key. {}", host, detail)),
            AiError::NoCredit(_) => Self::new(false, "no_credit", format!("{} accepted the key but reported no credit or a rate limit. {}", host, detail)),
            _ => Self::new(false, "other", detail),
        }
    }
}

/// The fixed probe body: one user word, a one-token answer, no meeting
/// content. `completion_tokens` switches to `max_completion_tokens`.
pub fn build_probe_request(model: &str, completion_tokens: bool) -> Value {
    let mut body = json!({"model": model, "messages": [{"role": "user", "content": "Hi"}], "stream": false});
    if completion_tokens {
        body["max_completion_tokens"] = json!(1);
    } else {
        body["max_tokens"] = json!(1);
    }
    body
}

/// Turn the probe's HTTP status and body into plain words. Pure; unit tested.
pub fn classify_probe(host: &str, model: &str, status: u16, body: &[u8], key: Option<&str>) -> ProbeOutcome {
    let msg = providers::redact_with(&error_message(body), key);
    let mentions_model = msg.to_ascii_lowercase().contains("model");
    match status {
        200..=299 => ProbeOutcome::new(true, "connected", format!("Connected to {} with model {}.", host, model)),
        401 | 403 => ProbeOutcome::new(
            false,
            "wrong_key",
            format!("{} rejected the API key ({}). Check the key and save it again.", host, status),
        ),
        402 | 429 => ProbeOutcome::new(
            false,
            "no_credit",
            format!("{} accepted the key but reported no credit or a rate limit ({}). {}", host, status, msg),
        ),
        300..=399 => ProbeOutcome::new(
            false,
            "bad_url",
            format!("{} redirected the request ({}); the base URL is probably wrong.", host, status),
        ),
        404 if !mentions_model => ProbeOutcome::new(
            false,
            "bad_url",
            format!("{} has no /chat/completions at this base URL (404). It usually ends in /v1.", host),
        ),
        400 | 404 | 422 if mentions_model => ProbeOutcome::new(
            false,
            "model_missing",
            format!("{} doesn't offer the model '{}'. {}", host, model, msg),
        ),
        _ => ProbeOutcome::new(false, "other", format!("{} answered {}: {}", host, status, msg)),
    }
}

fn host_of(base_url: &str) -> String {
    url::Url::parse(base_url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| base_url.to_string())
}

/// Send the one-token hello to a resolved target. Network failures become
/// outcomes rather than errors so the UI always gets plain words.
pub async fn probe(t: &Target) -> ProbeOutcome {
    let host = host_of(&t.base_url);
    let key = t.key.as_deref();
    match t.protocol {
        Protocol::Apple => ProbeOutcome::new(true, "connected", "Apple on-device model is available.".into()),
        Protocol::Anthropic => {
            let url = format!("{}/messages", t.base_url);
            let headers = request_headers(Protocol::Anthropic, &t.provider, key);
            let body = build_anthropic_request(&t.model, &[Msg::user("Hi")], 1, None);
            let req = HTTP.post(&url).json(&body).timeout(Duration::from_secs(30));
            match send(req, &headers, key).await {
                Ok((status, bytes)) => classify_probe(&host, &t.model, status, &bytes, key),
                Err(e) => ProbeOutcome::from_error(&e, &host),
            }
        }
        Protocol::OpenAI => {
            let url = format!("{}/chat/completions", t.base_url);
            let headers = request_headers(Protocol::OpenAI, &t.provider, key);
            let mut completion_tokens = t.completion_tokens;
            for attempt in 0..2 {
                let body = build_probe_request(&t.model, completion_tokens);
                let req = HTTP.post(&url).json(&body).timeout(Duration::from_secs(30));
                let (status, bytes) = match send(req, &headers, key).await {
                    Ok(r) => r,
                    Err(e) => return ProbeOutcome::from_error(&e, &host),
                };
                if attempt == 0 && !completion_tokens && needs_completion_tokens_retry(status, &String::from_utf8_lossy(&bytes)) {
                    completion_tokens = true;
                    continue;
                }
                return classify_probe(&host, &t.model, status, &bytes, key);
            }
            ProbeOutcome::new(false, "other", format!("{} rejected the request twice.", host))
        }
    }
}

/// List models (also the key validation step). `key` None for local servers.
pub async fn list_models(provider: &str, base_url: &str, key: Option<&str>) -> Result<Vec<config::ModelInfo>, AiError> {
    let p = preset(provider).ok_or_else(|| AiError::Other(format!("Unknown provider '{}'", provider)))?;
    if p.protocol == Protocol::Apple {
        let (ok, reason) = crate::store::apple_model_availability();
        if !ok {
            return Err(AiError::Unreachable(format!("Apple on-device model unavailable: {}", reason)));
        }
        return Ok(vec![config::ModelInfo {
            id: providers::APPLE_MODEL.to_string(),
            context: Some(providers::APPLE_CONTEXT_TOKENS),
            vision: Some(false),
        }]);
    }
    let base = providers::check_base_url(base_url).map_err(AiError::BadUrl)?;
    let (url, headers) = match p.protocol {
        Protocol::Apple => unreachable!(),
        Protocol::OpenAI => (format!("{}/models", base), request_headers(Protocol::OpenAI, provider, key)),
        Protocol::Anthropic => (
            format!("{}/models?limit=1000", base),
            request_headers(Protocol::Anthropic, provider, key),
        ),
    };
    let req = HTTP.get(&url).timeout(Duration::from_secs(20));
    let (status, bytes) = send(req, &headers, key).await?;
    if status == 404 {
        return Err(AiError::Other("404: no models endpoint here; check the base URL (it usually ends in /v1)".into()));
    }
    if !(200..300).contains(&status) {
        return Err(status_error(status, &bytes, key));
    }
    Ok(parse_models(&parse_json(&bytes)?))
}

pub fn parse_models(v: &Value) -> Vec<config::ModelInfo> {
    let items: Vec<Value> = match v {
        Value::Array(a) => a.clone(),
        _ => v["data"]
            .as_array()
            .or_else(|| v["models"].as_array())
            .cloned()
            .unwrap_or_default(),
    };
    let mut out: Vec<config::ModelInfo> = items
        .iter()
        .filter(|m| match m["type"].as_str() {
            // Together tags each model; keep only chat/language models
            Some(t) => matches!(t, "chat" | "language" | "model"),
            None => true,
        })
        .filter_map(|m| {
            let id = m["id"].as_str().or_else(|| m["name"].as_str())?;
            let id = id.strip_prefix("models/").unwrap_or(id).to_string();
            Some(config::ModelInfo {
                id,
                context: m["max_input_tokens"]
                    .as_u64()
                    .or_else(|| m["context_length"].as_u64())
                    .or_else(|| m["context_window"].as_u64())
                    .map(|n| n as usize),
                vision: m.pointer("/capabilities/image_input/supported").and_then(|b| b.as_bool()),
            })
        })
        .collect();
    let mut seen = std::collections::HashSet::new();
    out.retain(|m| seen.insert(m.id.clone()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msgs() -> Vec<Msg> {
        vec![
            Msg::system("You are helpful."),
            Msg::system("Meeting content: hello"),
            Msg::user("Summarize"),
        ]
    }

    #[test]
    fn openai_request_shape() {
        let b = build_openai_request("gpt-4.1-mini", &msgs(), 1500, Some(0.3), false);
        assert_eq!(b["max_tokens"], 1500);
        assert_eq!(b["temperature"].as_f64().unwrap() as f32, 0.3);
        assert_eq!(b["stream"], false);
        assert!(b.get("response_format").is_none());
        assert!(b.get("max_completion_tokens").is_none());
        assert_eq!(b["messages"][0]["role"], "system");
        assert_eq!(b["messages"][2]["content"], "Summarize");

        let b = build_openai_request("gpt-5-mini", &msgs(), 1500, Some(0.3), true);
        assert!(b.get("max_tokens").is_none());
        assert!(b.get("temperature").is_none());
        assert_eq!(b["max_completion_tokens"], 1500 + REASONING_HEADROOM);
    }

    #[test]
    fn openai_image_parts() {
        let m = vec![Msg::user_with_image("What app?", "image/jpeg", "QUJD".into())];
        let b = build_openai_request("gpt-4o-mini", &m, 800, Some(0.1), false);
        let c = &b["messages"][0]["content"];
        assert_eq!(c[0]["type"], "image_url");
        assert_eq!(c[0]["image_url"]["url"], "data:image/jpeg;base64,QUJD");
        assert_eq!(c[1]["type"], "text");
    }

    #[test]
    fn anthropic_request_shape() {
        let b = build_anthropic_request("claude-haiku-4-5", &msgs(), 1500, Some(0.3));
        assert_eq!(b["system"], "You are helpful.\n\nMeeting content: hello");
        assert_eq!(b["max_tokens"], 1500);
        assert_eq!(b["messages"].as_array().unwrap().len(), 1);
        assert_eq!(b["messages"][0]["role"], "user");
        assert_eq!(b["messages"][0]["content"][0]["text"], "Summarize");
        assert!(b.get("temperature").is_some());
        assert!(b.get("response_format").is_none());
        // Newer models: no sampling params, thinking headroom
        let b = build_anthropic_request("claude-opus-5", &msgs(), 1500, Some(0.3));
        assert!(b.get("temperature").is_none());
        assert_eq!(b["max_tokens"], 1500 + REASONING_HEADROOM);
    }

    #[test]
    fn anthropic_alternation_and_images() {
        let m = vec![
            Msg::new(Role::Assistant, "Earlier answer"),
            Msg::user("a"),
            Msg::user_with_image("b", "image/png", "QUJD".into()),
        ];
        let b = build_anthropic_request("claude-haiku-4-5", &m, 100, None);
        let turns = b["messages"].as_array().unwrap();
        assert_eq!(turns[0]["role"], "user");
        assert_eq!(turns[1]["role"], "assistant");
        assert_eq!(turns[2]["role"], "user");
        let img = &turns[2]["content"][1];
        assert_eq!(img["type"], "image");
        assert_eq!(img["source"]["type"], "base64");
        assert_eq!(img["source"]["media_type"], "image/png");
        assert!(b.get("system").is_none());
    }

    #[test]
    fn headers_per_protocol() {
        let h = request_headers(Protocol::OpenAI, "openai", Some("sk-x"));
        assert!(h.contains(&("authorization", "Bearer sk-x".into())));
        assert!(!h.iter().any(|(k, _)| *k == "x-api-key"));
        let h = request_headers(Protocol::OpenAI, "ollama", None);
        assert!(!h.iter().any(|(k, _)| *k == "authorization"));
        let h = request_headers(Protocol::Anthropic, "anthropic", Some("sk-ant-x"));
        assert!(h.contains(&("x-api-key", "sk-ant-x".into())));
        assert!(h.contains(&("anthropic-version", "2023-06-01".into())));
        assert!(!h.iter().any(|(k, _)| *k == "authorization"));
    }

    #[test]
    fn completion_tokens_retry_decision() {
        assert!(needs_completion_tokens_retry(
            400,
            r#"{"error":{"message":"Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead."}}"#
        ));
        assert!(needs_completion_tokens_retry(
            400,
            r#"{"error":{"message":"Unsupported value: 'temperature' does not support 0.3 with this model. Only the default (1) value is supported."}}"#
        ));
        assert!(!needs_completion_tokens_retry(400, r#"{"error":{"message":"model not found"}}"#));
        assert!(!needs_completion_tokens_retry(401, "max_tokens"));
        assert!(prefers_completion_tokens("openai", "gpt-5-mini"));
        assert!(prefers_completion_tokens("openai", "o3"));
        assert!(!prefers_completion_tokens("openai", "gpt-4.1-mini"));
        assert!(!prefers_completion_tokens("groq", "gpt-5"));
    }

    #[test]
    fn context_fit_trims_middle() {
        let transcript = format!("START {} END", "word ".repeat(60_000));
        let mut m = vec![Msg::system("Summarize."), Msg::user(transcript)];
        assert!(fit_messages(&mut m, 32_768, 1500));
        let budget = ((32_768 - 1500 - 512) as f64 * CHARS_PER_TOKEN) as usize;
        let total: usize = m.iter().map(|x| x.text().chars().count()).sum();
        assert!(total <= budget, "{} > {}", total, budget);
        let body = m[1].text();
        assert!(body.starts_with("START") && body.ends_with("END"));
        assert!(body.contains("trimmed"));
        assert_eq!(m[0].text(), "Summarize.");
        let mut short = vec![Msg::user("hello")];
        assert!(!fit_messages(&mut short, 8_192, 1500));
    }

    #[test]
    fn parses_responses() {
        let v = json!({"choices":[{"message":{"content":"<think>hmm</think>\nHi"},"finish_reason":"stop"}]});
        assert_eq!(parse_openai_response(&v).unwrap(), "Hi");
        let v = json!({"choices":[{"message":{"content":""},"finish_reason":"length"}]});
        assert_eq!(parse_openai_response(&v), Err(AiError::Truncated));
        let v = json!({"content":[{"type":"thinking","thinking":""},{"type":"text","text":"Hello"}],"stop_reason":"end_turn"});
        assert_eq!(parse_anthropic_response(&v).unwrap(), "Hello");
        let v = json!({"content":[],"stop_reason":"refusal"});
        assert!(parse_anthropic_response(&v).is_err());
    }

    #[test]
    fn parses_model_lists() {
        let openai = json!({"object":"list","data":[{"id":"gpt-4o-mini"},{"id":"models/gemini-2.5-flash"}]});
        let ids: Vec<String> = parse_models(&openai).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["gpt-4o-mini", "gemini-2.5-flash"]);
        let together = json!([{"id":"a","type":"chat"},{"id":"b","type":"embedding"}]);
        assert_eq!(parse_models(&together).len(), 1);
        let anthropic = json!({"data":[{"id":"claude-haiku-4-5","max_input_tokens":200000,"capabilities":{"image_input":{"supported":true}}}],"has_more":false});
        let m = &parse_models(&anthropic)[0];
        assert_eq!(m.context, Some(200_000));
        assert_eq!(m.vision, Some(true));
    }

    #[test]
    fn errors_are_classified_and_redacted() {
        let e = status_error(401, br#"{"error":{"message":"Incorrect API key provided: sk-proj-ABCDEFGHIJKLMNOPQRST"}}"#, None);
        assert_eq!(e.class(), "wrong_key");
        assert!(!e.to_string().contains("ABCDEFGHIJKLMNOPQRST"));
        assert_eq!(status_error(429, b"slow down", None).class(), "no_credit");
        assert_eq!(status_error(402, b"", None).class(), "no_credit");
        assert_eq!(status_error(500, b"oops", None).class(), "other");
        let e = status_error(400, b"bad key my-weird-secret-99", Some("my-weird-secret-99"));
        assert!(!e.to_string().contains("my-weird-secret-99"));
        assert_eq!(AiError::ConsentRequired("openai".into()).to_string(), "CONSENT_REQUIRED:openai");
    }

    #[test]
    fn custom_requests_need_an_explicit_url_and_remote_consent() {
        let mut c = config::AiConfig::default();
        assert!(matches!(resolve_for(&c, "custom", "user-model"), Err(AiError::BadUrl(_))));
        for legacy in ["gemini", "openai", "deepgram", "ollama"] {
            c.provider_mut(legacy).consent = true;
            c.provider_mut(legacy).base_url = Some("https://legacy.example.com/v1".into());
            assert!(resolve_for(&c, legacy, "saved-model").is_err());
        }
        c.provider_mut("custom").base_url = Some("http://127.0.0.1:8000/v1".into());
        assert_eq!(resolve_for(&c, "custom", "user-model").unwrap().base_url, "http://127.0.0.1:8000/v1");
        c.provider_mut("custom").base_url = Some("https://user.example.com/v1".into());
        assert_eq!(resolve_for(&c, "custom", "user-model").unwrap_err(), AiError::ConsentRequired("custom".into()));
        c.provider_mut("custom").consent = true;
        assert_eq!(resolve_for(&c, "custom", "user-model").unwrap().base_url, "https://user.example.com/v1");
    }

    #[test]
    fn timeouts_scale_with_max_tokens() {
        assert_eq!(request_timeout(0), Duration::from_secs(60));
        assert_eq!(request_timeout(1600), Duration::from_secs(260));
    }

    #[test]
    fn preset_urls_get_their_service_quirks_without_consent_bypass() {
        let mut c = config::AiConfig::default();
        c.provider_mut("custom").base_url = Some("https://api.openai.com/v1".into());
        // Real use still needs consent for a preset (it is a public endpoint)
        assert_eq!(resolve_for(&c, "custom", "gpt-6-luna").unwrap_err(), AiError::ConsentRequired("custom".into()));
        // The connection test resolves without consent but with the same URL and key rules
        let t = resolve_for_probe(&c, "custom", "gpt-6-luna").unwrap();
        assert_eq!(t.hint, "openai");
        assert!(t.completion_tokens, "gpt-6 wants max_completion_tokens");
        assert_eq!(t.context_tokens, 128_000);
        c.provider_mut("custom").base_url = Some("https://api.anthropic.com/v1".into());
        let t = resolve_for_probe(&c, "custom", "claude-sonnet-5-5").unwrap();
        assert_eq!(t.hint, "anthropic");
        assert!(!t.completion_tokens);
        c.provider_mut("custom").base_url = Some("https://proxy.example.com/v1".into());
        let t = resolve_for_probe(&c, "custom", "gpt-6-luna").unwrap();
        assert_eq!(t.hint, "custom");
        assert!(!t.completion_tokens);
        assert_eq!(t.context_tokens, 32_768);
        // A bad URL is still refused for the probe
        c.provider_mut("custom").base_url = Some("http://public.example.com/v1".into());
        assert!(matches!(resolve_for_probe(&c, "custom", "m"), Err(AiError::BadUrl(_))));
    }

    #[test]
    fn probe_body_is_one_fixed_word_and_one_token() {
        let b = build_probe_request("gpt-6-luna", false);
        assert_eq!(b["max_tokens"], 1);
        assert_eq!(b["messages"].as_array().unwrap().len(), 1);
        assert_eq!(b["messages"][0]["content"], "Hi");
        assert!(b.get("temperature").is_none());
        let b = build_probe_request("gpt-6-luna", true);
        assert_eq!(b["max_completion_tokens"], 1);
        assert!(b.get("max_tokens").is_none());
    }

    #[test]
    fn probe_outcomes_are_plain_words_and_redacted() {
        let ok = classify_probe("api.example.com", "m1", 200, br#"{"choices":[]}"#, None);
        assert!(ok.ok);
        assert_eq!(ok.class, "connected");
        assert!(ok.message.contains("api.example.com") && ok.message.contains("m1"));
        let e = classify_probe("api.example.com", "m1", 401, br#"{"error":{"message":"Incorrect API key provided: sk-proj-ABCDEFGHIJKLMNOPQRST"}}"#, None);
        assert_eq!((e.ok, e.class), (false, "wrong_key"));
        assert!(!e.message.contains("ABCDEFGHIJKLMNOPQRST"));
        assert_eq!(classify_probe("h", "m", 403, b"", None).class, "wrong_key");
        assert_eq!(classify_probe("h", "m", 429, b"slow down", None).class, "no_credit");
        assert_eq!(classify_probe("h", "m", 402, b"", None).class, "no_credit");
        assert_eq!(classify_probe("h", "m", 404, b"<html>not here</html>", None).class, "bad_url");
        assert_eq!(classify_probe("h", "m", 404, br#"{"error":{"message":"The model `m` does not exist"}}"#, None).class, "model_missing");
        assert_eq!(classify_probe("h", "m", 400, br#"{"error":{"message":"invalid model"}}"#, None).class, "model_missing");
        assert_eq!(classify_probe("h", "m", 302, b"", None).class, "bad_url");
        assert_eq!(classify_probe("h", "m", 500, b"oops", None).class, "other");
        let e = ProbeOutcome::from_error(&AiError::NoKey("x".into()), "h");
        assert_eq!(e.class, "no_key");
        let e = ProbeOutcome::from_error(&AiError::Unreachable("dns failed".into()), "h");
        assert_eq!(e.class, "unreachable");
        assert!(e.message.contains("h") && e.message.contains("dns failed"));
    }

    /// Loopback stub that answers one request with a canned status/body.
    fn stub_server(status: u16, body: &'static str) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut buf = Vec::new();
            let mut tmp = [0u8; 4096];
            loop {
                let n = s.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
                    let len: usize = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    if buf.len() >= end + 4 + len {
                        // Assert the probe carries no meeting content
                        let sent = String::from_utf8_lossy(&buf[end + 4..]);
                        assert!(sent.contains("\"Hi\""), "{}", sent);
                        assert!(!sent.contains("transcript"), "{}", sent);
                        break;
                    }
                }
            }
            let reason = match status {
                200 => "OK",
                401 => "Unauthorized",
                _ => "Error",
            };
            let resp = format!(
                "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status,
                reason,
                body.len(),
                body
            );
            let _ = s.write_all(resp.as_bytes());
            let _ = s.flush();
        });
        format!("http://127.0.0.1:{}/v1", port)
    }

    // ── Streaming ────────────────────────────────────────────────────────

    #[test]
    fn a_stream_is_read_in_whatever_pieces_arrive() {
        let body = concat!(
            ": keep-alive\n\n",
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"hmm\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Ship \"}}]}\r\n\r\n",
            "data: {\"choices\":[{\"delta\":{\"content\":[{\"type\":\"text\",\"text\":\"Fri\"}]}}]}\n\n",
            "data:{\"choices\":[{\"delta\":{\"content\":\"day ✓\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"total_tokens\":9}}\n\n",
            "data: [DONE]\n\n",
        )
        .as_bytes();
        // Whole, byte by byte (a multi-byte character split across reads), and in odd chunks
        for size in [body.len(), 1, 7, 64] {
            let mut sse = SseText::new(Protocol::OpenAI);
            let mut grew = 0;
            for piece in body.chunks(size) {
                if sse.feed(piece) {
                    grew += 1;
                }
            }
            sse.end();
            assert_eq!(sse.text, "Ship Friday ✓", "chunks of {}", size);
            assert!(sse.done && sse.error.is_none());
            assert_eq!(sse.finish.as_deref(), Some("stop"));
            assert!(grew >= 1);
            assert_eq!(sse.finish(None).unwrap(), "Ship Friday ✓");
        }
        // A last line without its newline still counts
        let mut sse = SseText::new(Protocol::OpenAI);
        sse.feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"End\"},\"finish_reason\":\"stop\"}]}");
        assert_eq!(sse.text, "");
        sse.end();
        assert_eq!(sse.finish(None).unwrap(), "End");
        // A body that just stops is a broken connection, never half an answer
        let mut cut = SseText::new(Protocol::OpenAI);
        cut.feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"They decided to\"}}]}\n\n");
        cut.end();
        assert!(matches!(cut.finish(None), Err(AiError::Unreachable(_))));
        // The end marker alone is enough (some servers send no finish reason)
        let mut marked = SseText::new(Protocol::OpenAI);
        marked.feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"Whole.\"}}]}\n\ndata: [DONE]\n\n");
        assert_eq!(marked.finish(None).unwrap(), "Whole.");
    }

    #[test]
    fn an_anthropic_stream_gives_its_text_and_skips_the_thinking() {
        let mut sse = SseText::new(Protocol::Anthropic);
        sse.feed(
            concat!(
                "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"content\":[]}}\n\n",
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"let me see\"}}\n\n",
                "event: ping\ndata: {\"type\":\"ping\"}\n\n",
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"Budget \"}}\n\n",
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"agreed.\"}}\n\n",
                "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            )
            .as_bytes(),
        );
        assert!(sse.done);
        assert_eq!(sse.finish(None).unwrap(), "Budget agreed.");

        // Cut off before any text: the same error as a whole response
        let mut cut = SseText::new(Protocol::Anthropic);
        cut.feed(b"data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"max_tokens\"}}\n\n");
        assert_eq!(cut.finish(None), Err(AiError::Truncated));
        let mut no = SseText::new(Protocol::Anthropic);
        no.feed(b"data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"refusal\"}}\n\n");
        assert!(matches!(no.finish(None), Err(AiError::Other(m)) if m.contains("declined")));
    }

    #[test]
    fn a_stream_that_fails_or_says_nothing_is_an_error_without_the_key() {
        let mut sse = SseText::new(Protocol::OpenAI);
        sse.feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"Half an\"}}]}\n\n");
        sse.feed(b"data: {\"error\":{\"message\":\"Rate limit for key sk-secret-1234567890 reached\"}}\n\n");
        match sse.finish(Some("sk-secret-1234567890")) {
            Err(AiError::Other(m)) => assert!(m.contains("Rate limit") && !m.contains("sk-secret-1234567890"), "{}", m),
            other => panic!("{:?}", other),
        }
        let mut err = SseText::new(Protocol::Anthropic);
        err.feed(b"event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n");
        assert_eq!(err.finish(None), Err(AiError::Other("Overloaded".into())));
        // An error that names its kind keeps it, as its HTTP status would have
        let mut rate = SseText::new(Protocol::Anthropic);
        rate.feed(b"data: {\"type\":\"error\",\"error\":{\"type\":\"rate_limit_error\",\"message\":\"Slow down\"}}\n\n");
        assert_eq!(rate.finish(None), Err(AiError::NoCredit("Slow down".into())));
        let mut auth = SseText::new(Protocol::OpenAI);
        auth.feed(b"data: {\"error\":{\"code\":\"invalid_api_key\",\"message\":\"Bad key\"}}\n\n");
        assert_eq!(auth.finish(None), Err(AiError::WrongKey("Bad key".into())));

        let mut empty = SseText::new(Protocol::OpenAI);
        empty.feed(b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n\n");
        assert_eq!(empty.finish(None), Err(AiError::Truncated));
        let mut nothing = SseText::new(Protocol::OpenAI);
        nothing.feed(b"data: [DONE]\n\n");
        assert!(matches!(nothing.finish(None), Err(AiError::Other(m)) if m.contains("empty answer")));
        // Only thinking: nothing to show, and nothing to return
        let mut think = SseText::new(Protocol::OpenAI);
        think.feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"<think>plan</think>\"},\"finish_reason\":\"length\"}]}\n\n");
        assert_eq!(think.finish(None), Err(AiError::Truncated));
    }

    #[test]
    fn thinking_is_never_shown_while_it_is_written() {
        assert_eq!(visible_so_far("Plain so far"), "Plain so far");
        assert_eq!(visible_so_far("<think>plan the ans"), "");
        assert_eq!(visible_so_far("<think>plan</think>Ship"), "Ship");
        assert_eq!(visible_so_far("Ship <think>hmm</think>Friday <think>more"), "Ship Friday");
        // A tag that hasn't finished arriving
        for partial in ["Ship <", "Ship <t", "Ship <thin", "Ship <think"] {
            assert_eq!(visible_so_far(partial), "Ship", "{}", partial);
        }
        // Other angle brackets are text
        assert_eq!(visible_so_far("a < b and <b"), "a < b and <b");
        assert_eq!(visible_so_far("x <thing"), "x <thing");
        // What is shown at the end is the answer
        let whole = "<think>plan</think>\nShip Friday.";
        assert_eq!(visible_so_far(whole), strip_think(whole));
        assert!(stream_refused(400, r#"{"error":{"message":"'stream' is not supported with this model"}}"#));
        assert!(!stream_refused(500, "stream"));
        assert!(!stream_refused(400, "max_tokens is too large"));
    }

    /// Loopback stub for whole completions: answers each connection in turn
    /// with `(status, content type, body pieces)`, the pieces a moment apart
    /// and the connection closed after the last. Returns the base URL and
    /// the request bodies it received.
    fn scripted_server(replies: Vec<(u16, &'static str, Vec<&'static str>)>) -> (String, std::sync::Arc<parking_lot::Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for (status, kind, pieces) in replies {
                let (mut s, _) = listener.accept().unwrap();
                s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                loop {
                    let n = s.read(&mut tmp).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
                        let len: usize = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse().ok())
                            .unwrap_or(0);
                        if buf.len() >= end + 4 + len {
                            log.lock().push(String::from_utf8_lossy(&buf[end + 4..]).into_owned());
                            break;
                        }
                    }
                }
                let _ = s.write_all(format!("HTTP/1.1 {} X\r\nContent-Type: {}\r\nConnection: close\r\n\r\n", status, kind).as_bytes());
                for piece in pieces {
                    let _ = s.write_all(piece.as_bytes());
                    let _ = s.flush();
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
        });
        (format!("http://127.0.0.1:{}/v1", port), seen)
    }

    fn sent(seen: &parking_lot::Mutex<Vec<String>>, n: usize) -> Value {
        serde_json::from_str(&seen.lock()[n]).unwrap()
    }

    #[tokio::test]
    async fn a_streamed_answer_reaches_the_sink_while_it_is_written() {
        let (url, seen) = scripted_server(vec![(
            200,
            "text/event-stream",
            vec![
                "data: {\"choices\":[{\"delta\":{\"content\":\"<think>pl\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"an</think>Ship\"}}]}\n\ndata: {\"choi",
                "ces\":[{\"delta\":{\"content\":\" Friday\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\".\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
            ],
        )]);
        let shown = parking_lot::Mutex::new(Vec::<String>::new());
        let out = run_stream(&target(url, Some("stub-key")), msgs(), Opts::default(), &|t| shown.lock().push(t.to_string())).await;
        assert_eq!(out.unwrap(), "Ship Friday.");
        let shown = shown.into_inner();
        assert_eq!(shown.last().map(String::as_str), Some("Ship Friday."));
        assert!(shown.len() >= 2, "one piece only: {:?}", shown);
        assert!(shown.iter().all(|t| !t.contains("think") && !t.contains("plan")), "{:?}", shown);
        assert!(shown.windows(2).all(|w| w[1].starts_with(&w[0]) && w[1] != w[0]), "{:?}", shown);
        let body = sent(&seen, 0);
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 1500);
        assert!(body.get("response_format").is_none());

        // Without a sink the same target is asked the plain way
        let (url, seen) = scripted_server(vec![(200, "application/json", vec![r#"{"choices":[{"message":{"content":"Whole."},"finish_reason":"stop"}]}"#])]);
        assert_eq!(run(&target(url, None), msgs(), Opts::default()).await.unwrap(), "Whole.");
        assert_eq!(sent(&seen, 0)["stream"], false);
    }

    #[tokio::test]
    async fn a_server_that_cannot_stream_still_answers() {
        // It ignores `stream` and answers with plain JSON (no event-stream header to go by)
        let (url, _) = scripted_server(vec![(200, "text/plain", vec![" \n", r#"{"choices":[{"message":{"content":"All at"#, r#" once."},"finish_reason":"stop"}]}"#])]);
        let shown = parking_lot::Mutex::new(Vec::<String>::new());
        let out = run_stream(&target(url, None), msgs(), Opts::default(), &|t| shown.lock().push(t.to_string())).await;
        assert_eq!(out.unwrap(), "All at once.");
        assert_eq!(shown.into_inner(), vec!["All at once."]);

        // It refuses `stream`: asked again without it, once
        let (url, seen) = scripted_server(vec![
            (400, "application/json", vec![r#"{"error":{"message":"stream=true is not supported"}}"#]),
            (200, "application/json", vec![r#"{"choices":[{"message":{"content":"Second try."},"finish_reason":"stop"}]}"#]),
        ]);
        let shown = parking_lot::Mutex::new(Vec::<String>::new());
        let out = run_stream(&target(url, None), msgs(), Opts::default(), &|t| shown.lock().push(t.to_string())).await;
        assert_eq!(out.unwrap(), "Second try.");
        assert_eq!(shown.into_inner(), vec!["Second try."]);
        assert_eq!((sent(&seen, 0)["stream"].clone(), sent(&seen, 1)["stream"].clone()), (json!(true), json!(false)));

        // The token-parameter retry still streams
        let (url, seen) = scripted_server(vec![
            (400, "application/json", vec![r#"{"error":{"message":"Unsupported parameter: 'max_tokens'. Use 'max_completion_tokens'."}}"#]),
            (200, "text/event-stream", vec!["data: {\"choices\":[{\"delta\":{\"content\":\"Ok.\"},\"finish_reason\":\"stop\"}]}\n\n"]),
        ]);
        let out = run_stream(&target(url, None), msgs(), Opts::default(), &|_| {}).await;
        assert_eq!(out.unwrap(), "Ok.");
        let second = sent(&seen, 1);
        assert_eq!(second["stream"], true);
        assert!(second.get("max_tokens").is_none() && second.get("max_completion_tokens").is_some());

        // An error status is the same error as ever, key redacted
        let (url, _) = scripted_server(vec![(401, "application/json", vec![r#"{"error":{"message":"Bad key stub-key-value-1234"}}"#])]);
        match run_stream(&target(url, Some("stub-key-value-1234")), msgs(), Opts::default(), &|_| {}).await {
            Err(AiError::WrongKey(m)) => assert!(!m.contains("stub-key-value-1234"), "{}", m),
            other => panic!("{:?}", other),
        }
    }

    #[tokio::test]
    async fn an_anthropic_answer_streams_from_the_messages_endpoint() {
        let (url, seen) = scripted_server(vec![(
            200,
            "text/event-stream; charset=utf-8",
            vec![
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Two \"}}\n\n",
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"sites.\"}}\n\n",
                "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            ],
        )]);
        let mut t = target(url, Some("stub-key"));
        t.protocol = Protocol::Anthropic;
        t.model = "claude-sonnet-4-5".into();
        let shown = parking_lot::Mutex::new(Vec::<String>::new());
        let out = run_stream(&t, msgs(), Opts::default(), &|s| shown.lock().push(s.to_string())).await;
        assert_eq!(out.unwrap(), "Two sites.");
        assert_eq!(shown.into_inner().last().map(String::as_str), Some("Two sites."));
        let body = sent(&seen, 0);
        assert_eq!(body["stream"], true);
        assert!(body["system"].is_string() && body["max_tokens"].is_number());
    }

    fn target(base_url: String, key: Option<&str>) -> Target {
        Target {
            provider: "custom".into(),
            hint: "custom",
            protocol: Protocol::OpenAI,
            base_url,
            key: key.map(String::from),
            model: "stub-model".into(),
            context_tokens: 32_768,
            completion_tokens: false,
        }
    }

    #[tokio::test]
    async fn probe_reports_success_auth_failure_and_network_error() {
        let ok = probe(&target(stub_server(200, r#"{"choices":[{"message":{"content":"Hi"},"finish_reason":"length"}]}"#), Some("stub-key-value-1234"))).await;
        assert_eq!((ok.ok, ok.class), (true, "connected"), "{}", ok.message);
        assert!(ok.message.contains("127.0.0.1"));

        let bad = probe(&target(stub_server(401, r#"{"error":{"message":"Incorrect API key provided: stub-key-value-1234"}}"#), Some("stub-key-value-1234"))).await;
        assert_eq!((bad.ok, bad.class), (false, "wrong_key"), "{}", bad.message);
        assert!(!bad.message.contains("stub-key-value-1234"));

        // A port nobody listens on
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let down = probe(&target(format!("http://127.0.0.1:{}/v1", port), None)).await;
        assert_eq!((down.ok, down.class), (false, "unreachable"), "{}", down.message);
        assert!(down.message.starts_with("Couldn't reach 127.0.0.1"));
    }
}
