//! The `OpenRouter` adapter: one HTTP POST to `chat/completions` per completion. It maps the HTTP
//! statuses to the provider errors so a caller can retry the transient ones.
//!
//! The same adapter serves a local OpenAI-compatible server such as llama.cpp: [`OpenRouter::local`]
//! points it at the URL from `RHIPPER_LOCAL_URL` with no key. [`LocalProbe`] asks that server for
//! its model list in the background, so a status dot can say whether it is up without a model call.
//!
//! Not every route prices its answers: a local server never does, and `OpenRouter` leaves the cost
//! out of some. [`Prices`] holds the list prices per token from the models endpoint, read once a
//! day into `var/prices.json`, and an answer without a cost is priced from them and marked as
//! computed in the ledger; a local model is free.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::{Completion, CompletionFuture, CompletionRequest, Message, ModelProvider, ProviderError, ToolCall, ToolSpec, Usage};

/// The API root of the public service.
const BASE_URL: &str = "https://openrouter.ai/api/v1";
/// The environment variable that names the local server's API root.
pub const LOCAL_URL_VAR: &str = "RHIPPER_LOCAL_URL";
/// The API root of a local server when the variable is unset: llama.cpp's default port.
pub const DEFAULT_LOCAL_URL: &str = "http://127.0.0.1:8080/v1";
/// What the requests say they are.
const USER_AGENT: &str = concat!("rhipper-agent/", env!("CARGO_PKG_VERSION"));
/// How long a probe of the local server waits: it answers from memory or not at all.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// How often the status dots of the console and the window probe the local server.
pub const PROBE_EVERY: Duration = Duration::from_mins(1);
/// How long the public model list may take: hundreds of models with their prices.
const LIST_TIMEOUT: Duration = Duration::from_secs(20);
/// Where a project keeps the prices, relative to its root.
pub const PRICES_FILE: &str = "var/prices.json";
/// How long a read of the prices serves before the models endpoint is asked again.
pub const PRICES_FOR: Duration = Duration::from_hours(24);

/// The `OpenRouter` provider.
pub struct OpenRouter {
    client: reqwest::Client,
    /// The bearer key; empty sends no `Authorization` header, as a local server needs none.
    key: String,
    /// The API root; the tests point it at a local server.
    base_url: String,
    /// Where the prices are kept between runs; `None` keeps them for this provider only.
    prices_path: Option<PathBuf>,
    /// The prices read so far, shared by the completions of this provider.
    prices: Mutex<Option<Arc<Prices>>>,
}

/// The list prices of the models a route offers, per token and in dollars, with the time they
/// were read. `OpenRouter`'s `models` endpoint names them; a local server lists none.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Prices {
    /// When the list was read, in seconds since the epoch.
    pub read_at: u64,
    /// The price of each model, by id.
    pub models: BTreeMap<String, Price>,
}

/// What one token costs a model, in dollars: read and written, and the cache's own rates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Price {
    pub prompt: f64,
    pub completion: f64,
    /// A prompt token served from the cache; `None` prices it as any prompt token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    /// A prompt token written to the cache; `None` prices it as any prompt token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write: Option<f64>,
}

/// The tokens of a call split by the rate each is priced at.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    /// The prompt tokens neither read from nor written to the cache.
    pub fresh: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
}

impl Price {
    /// An Anthropic list price from its dollars per million tokens in and out; the cache reads
    /// at a tenth of the input and writes, for the five minutes the harness asks, at a
    /// quarter more.
    const fn anthropic(input: f64, output: f64) -> Self {
        let per_token = 1e-6;
        Self {
            prompt: input * per_token,
            completion: output * per_token,
            cache_read: Some(input * per_token * 0.1),
            cache_write: Some(input * per_token * 1.25),
        }
    }

    /// The cost of `tokens` at this price.
    #[must_use]
    pub fn of(&self, tokens: &Tokens) -> f64 {
        // the token counts are far below 2^53, so the conversion is exact
        #[allow(clippy::cast_precision_loss)]
        let n = |n: u64| n as f64;
        n(tokens.fresh) * self.prompt
            + n(tokens.cache_read) * self.cache_read.unwrap_or(self.prompt)
            + n(tokens.cache_write) * self.cache_write.unwrap_or(self.prompt)
            + n(tokens.output) * self.completion
    }
}

/// The Anthropic models' list prices, by the words of a model id, the first match standing: the
/// Claude Code route reports a cost only with a result, and a run stopped before one is priced
/// here; `OpenRouter` falls back to it for a Claude model its list leaves out.
const ANTHROPIC: [(&str, Price); 15] = [
    // Fable 5.2 lowered Fable's price; 5.0 and 5.1 keep theirs (2026-11-13 list)
    ("fable-5-2", Price::anthropic(8.0, 40.0)),
    ("fable-5.2", Price::anthropic(8.0, 40.0)),
    ("fable", Price::anthropic(10.0, 50.0)),
    ("opus-5-5", Price::anthropic(4.0, 20.0)),
    ("opus-5.5", Price::anthropic(4.0, 20.0)),
    ("sonnet-5-5", Price::anthropic(2.0, 10.0)),
    ("sonnet-5.5", Price::anthropic(2.0, 10.0)),
    ("opus-4-0", Price::anthropic(15.0, 75.0)),
    ("opus-4-1", Price::anthropic(15.0, 75.0)),
    ("opus-4.1", Price::anthropic(15.0, 75.0)),
    ("opus-4-2025", Price::anthropic(15.0, 75.0)),
    ("3-opus", Price::anthropic(15.0, 75.0)),
    ("opus", Price::anthropic(5.0, 25.0)),
    ("sonnet", Price::anthropic(3.0, 15.0)),
    ("haiku", Price::anthropic(1.0, 5.0)),
];

/// The Anthropic list price of `model`, from its id or its alias (`opus`, `claude-sonnet-4-6`,
/// `anthropic/claude-haiku-4.5`); `None` for any other model.
#[must_use]
pub fn anthropic_price(model: &str) -> Option<Price> {
    let model = model.to_ascii_lowercase();
    ANTHROPIC.iter().find(|(words, _)| model.contains(words)).map(|(_, price)| *price)
}

impl Prices {
    /// The prices in the file at `path`; `None` when there is no file or it is not a price list.
    #[must_use]
    pub fn load(path: &Path) -> Option<Self> {
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }

    /// Writes the prices to `path`.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(path, serde_json::to_string_pretty(self).map_err(std::io::Error::other)?)
    }

    /// Whether the list was read within [`PRICES_FOR`] of `now`.
    #[must_use]
    pub fn fresh_at(&self, now: u64) -> bool {
        now.saturating_sub(self.read_at) < PRICES_FOR.as_secs()
    }

    /// The price of `model`: the list read from the route, else the Anthropic list for a Claude
    /// model; `None` for a model neither names.
    #[must_use]
    pub fn price_of(&self, model: &str) -> Option<Price> {
        self.models.get(model).copied().or_else(|| anthropic_price(model))
    }

    /// The list price of `usage` on `model`: the cached prompt tokens at the cache-read price,
    /// the rest at the prompt price, every completion token at the completion price; `None`
    /// for a model not listed.
    #[must_use]
    pub fn cost(&self, model: &str, usage: &Usage) -> Option<f64> {
        let tokens = Tokens {
            fresh: usage.prompt_tokens.saturating_sub(usage.cached_tokens),
            cache_read: usage.cached_tokens,
            cache_write: 0,
            output: usage.completion_tokens,
        };
        Some(self.price_of(model)?.of(&tokens))
    }

    /// The prices of the models the endpoint listed, at `now`; a model without a parsable price
    /// is left out, and prices from a list without none, as a local server's, are an empty list.
    fn from_wire(models: Vec<WireModel>, now: u64) -> Self {
        let models = models
            .into_iter()
            .filter_map(|m| {
                let pricing = m.pricing?;
                let per_token = |text: Option<String>| text?.trim().parse::<f64>().ok();
                Some((
                    m.id,
                    Price {
                        prompt: per_token(pricing.prompt)?,
                        completion: per_token(pricing.completion)?,
                        cache_read: per_token(pricing.input_cache_read),
                        cache_write: per_token(pricing.input_cache_write),
                    },
                ))
            })
            .collect();
        Self { read_at: now, models }
    }
}

/// Seconds since the epoch, 0 on a clock set before it.
fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl OpenRouter {
    /// Builds the provider from an `OpenRouter` API key.
    ///
    /// # Errors
    /// When the HTTP client cannot be built.
    pub fn new(key: impl Into<String>) -> anyhow::Result<Self> {
        Self::at(BASE_URL, key)
    }

    /// Builds the provider against the local server at [`local_url`], with no key.
    ///
    /// # Errors
    /// When the HTTP client cannot be built.
    pub fn local() -> anyhow::Result<Self> {
        Self::at(&local_url(), "")
    }

    /// Builds the provider against another base URL, for tests and local servers. A trailing slash
    /// is dropped, so `http://host/v1/` and `http://host/v1` reach the same endpoint.
    ///
    /// # Errors
    /// When the HTTP client cannot be built.
    pub fn at(base_url: &str, key: impl Into<String>) -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder().user_agent(USER_AGENT).build()?,
            key: key.into(),
            base_url: base_url.trim_end_matches('/').to_owned(),
            prices_path: None,
            prices: Mutex::new(None),
        })
    }

    /// Keeps the prices in the file at `path` between runs: a fresh file is read instead of the
    /// endpoint, and a read of the endpoint is written there when its directory exists.
    #[must_use]
    pub fn with_prices_at(mut self, path: impl Into<PathBuf>) -> Self {
        self.prices_path = Some(path.into());
        self
    }

    /// The API root the provider posts to.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Whether the key is empty: the sign of a local server, which has no accounts.
    fn is_local(&self) -> bool {
        self.key.is_empty()
    }

    /// The ids the server offers, from `GET models`. A local server lists the files it loaded;
    /// `ping` names them when a model is unknown to it.
    ///
    /// # Errors
    /// When the server does not answer, or answers with something other than a model list.
    pub async fn models(&self) -> Result<Vec<String>, ProviderError> {
        Ok(self.fetch_models().await?.into_iter().map(|m| m.id).collect())
    }

    /// The models endpoint as it answers: the ids, and the prices where the route has them. A
    /// local server answers from memory within the probe's timeout; the public list takes longer.
    async fn fetch_models(&self) -> Result<Vec<WireModel>, ProviderError> {
        let url = format!("{}/models", self.base_url);
        let mut get = self.client.get(&url).timeout(if self.is_local() { PROBE_TIMEOUT } else { LIST_TIMEOUT });
        if !self.is_local() {
            get = get.bearer_auth(&self.key);
        }
        let response = get.send().await.map_err(|e| ProviderError::Unavailable(format!("cannot reach {url}: {e}")))?;
        let status = response.status();
        let body = response.text().await.map_err(|e| ProviderError::Unavailable(format!("{url} dropped the answer: {e}")))?;
        if status != StatusCode::OK {
            return Err(ProviderError::Other(error_text(&body, status)));
        }
        let parsed: WireModels = serde_json::from_str(&body)
            .map_err(|e| ProviderError::Other(format!("unparsable model list: {e}; body starts with: {}", snippet(&body))))?;
        Ok(parsed.data)
    }

    /// The prices of the route: the ones read within the day, from memory or the file, else a
    /// read of the endpoint, written to the file when there is one. When the endpoint cannot be
    /// read, a stale list serves rather than none, and `None` is only no list at all.
    pub async fn prices(&self) -> Option<Arc<Prices>> {
        let now = now();
        let held = self.prices.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        if let Some(prices) = &held
            && prices.fresh_at(now)
        {
            return Some(Arc::clone(prices));
        }
        let on_disk = self.prices_path.as_deref().and_then(Prices::load).map(Arc::new);
        if let Some(prices) = &on_disk
            && prices.fresh_at(now)
        {
            *self.prices.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(prices));
            return Some(Arc::clone(prices));
        }
        let fetched = match self.fetch_models().await {
            Ok(models) => Arc::new(Prices::from_wire(models, now)),
            Err(e) => {
                tracing::warn!("the prices cannot be read from {}: {e}", self.base_url);
                return held.or(on_disk);
            }
        };
        if let Some(path) = &self.prices_path
            && path.parent().is_none_or(|dir| dir.as_os_str().is_empty() || dir.exists())
            && let Err(e) = fetched.save(path)
        {
            tracing::warn!("the prices cannot be written to {}: {e}", path.display());
        }
        *self.prices.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&fetched));
        Some(fetched)
    }

    /// Fills in the cost of an answer the route did not price: nothing on a local server, the
    /// list price of the tokens on `OpenRouter`; the usage says the cost was computed. An answer
    /// on a model without a listed price keeps no cost.
    async fn price(&self, model: &str, usage: &mut Usage) {
        let cost = if self.is_local() { Some(0.0) } else { self.prices().await.and_then(|p| p.cost(model, usage)) };
        if let Some(cost) = cost {
            usage.cost_usd = Some(cost);
            usage.cost_computed = true;
        }
    }

    /// The text of an unknown-model error. On a local server it goes on to name what the server
    /// offers, since the id was typed, not picked from a list.
    async fn unknown_model_text(&self, body: &str, status: StatusCode) -> String {
        let mut text = error_text(body, status);
        if self.is_local()
            && let Ok(models) = self.models().await
            && !models.is_empty()
        {
            use std::fmt::Write as _;
            let _ = write!(text, "; the server at {} offers: {}", self.base_url, models.join(", "));
        }
        text
    }
}

/// The availability of the local server, as the status dots show it: probed in the background
/// with one `GET models` and no model call, shared between the probe and the frame that reads it.
#[derive(Clone, Default)]
pub struct LocalProbe {
    /// 0 unprobed, 1 down, 2 up.
    state: Arc<AtomicU8>,
}

impl LocalProbe {
    /// Whether the server answered the last probe; `None` before the first one answers.
    pub fn up(&self) -> Option<bool> {
        match self.state.load(Ordering::Relaxed) {
            0 => None,
            1 => Some(false),
            _ => Some(true),
        }
    }

    /// One probe of the server at `base_url`, to spawn on the runtime; it writes the outcome
    /// where [`up`](Self::up) reads it.
    pub async fn probe(self, base_url: &str) {
        let answered = match OpenRouter::at(base_url, "") {
            Ok(server) => server.models().await.is_ok(),
            Err(_) => false,
        };
        self.state.store(if answered { 2 } else { 1 }, Ordering::Relaxed);
    }

    /// [`probe`](Self::probe) at [`local_url`].
    pub async fn probe_local(self) {
        self.probe(&local_url()).await;
    }
}

#[derive(Deserialize)]
struct WireModels {
    #[serde(default)]
    data: Vec<WireModel>,
}

#[derive(Deserialize)]
struct WireModel {
    id: String,
    /// The list prices, which OpenRouter sends as decimal strings; a local server sends none.
    #[serde(default)]
    pricing: Option<WirePricing>,
}

#[derive(Deserialize)]
struct WirePricing {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
    /// The cache's rates, for the models that list them.
    #[serde(default)]
    input_cache_read: Option<String>,
    #[serde(default)]
    input_cache_write: Option<String>,
}

/// The API root of the local server: `RHIPPER_LOCAL_URL` when it is set to something, else
/// [`DEFAULT_LOCAL_URL`].
pub fn local_url() -> String {
    std::env::var(LOCAL_URL_VAR).ok().map(|u| u.trim().to_owned()).filter(|u| !u.is_empty()).unwrap_or_else(|| DEFAULT_LOCAL_URL.to_owned())
}

/// The wire format of one chat message. Content is optional because tool messages may carry it
/// as null and reasoning models answer in `reasoning` instead.
#[derive(serde::Serialize, Deserialize)]
struct WireMessage<'a> {
    role: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<&'a str>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<WireToolCall<'a>>,
    /// The id of the call a tool message answers; `tool_call_id` on the wire.
    #[serde(rename = "tool_call_id", skip_serializing_if = "Option::is_none")]
    answering: Option<&'a str>,
}

#[derive(serde::Serialize, Deserialize)]
struct WireToolCall<'a> {
    id: &'a str,
    /// The call kind; OpenRouter expects `function`.
    #[serde(rename = "type")]
    kind: &'a str,
    function: WireFunction<'a>,
}

#[derive(serde::Serialize, Deserialize)]
struct WireFunction<'a> {
    name: &'a str,
    /// The arguments, a JSON object in text form.
    arguments: &'a str,
}

#[derive(serde::Serialize)]
struct WireTool<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    function: WireToolFunction<'a>,
}

#[derive(serde::Serialize)]
struct WireToolFunction<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a serde_json::Value,
}

#[derive(serde::Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool<'a>>,
}

#[derive(Deserialize)]
struct WireResponse {
    choices: Vec<WireChoice>,
    #[serde(default)]
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct WireChoice {
    message: WireMessageOwned,
}

/// The message of an answer. A reasoning model answers in `reasoning` and leaves `content` null or
/// empty, so both fields are optional and the text takes whichever one holds something.
#[derive(Deserialize)]
struct WireMessageOwned {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Vec<WireToolCallOwned>,
}

#[derive(Deserialize)]
struct WireToolCallOwned {
    id: String,
    function: WireFunctionOwned,
}

#[derive(Deserialize)]
struct WireFunctionOwned {
    name: String,
    #[serde(default)]
    arguments: Option<String>,
}

impl WireMessageOwned {
    /// The text of the answer: the content, else the reasoning.
    fn text(&self) -> String {
        self.content.clone().filter(|c| !c.trim().is_empty()).or_else(|| self.reasoning.clone()).unwrap_or_default()
    }

    /// The tool calls of the answer, with the arguments defaulting to an empty object.
    fn tool_calls(&self) -> Vec<ToolCall> {
        self.tool_calls
            .iter()
            .map(|c| ToolCall {
                id: c.id.clone(),
                name: c.function.name.clone(),
                arguments: c.function.arguments.clone().unwrap_or_else(|| "{}".into()),
            })
            .collect()
    }
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    #[serde(rename = "prompt_tokens_details")]
    prompt_tokens_details: Option<PromptTokensDetails>,
    /// The cost in credits, which OpenRouter prices in dollars.
    #[serde(default)]
    cost: Option<f64>,
}

#[derive(Deserialize)]
struct PromptTokensDetails {
    #[serde(default, rename = "cached_tokens")]
    cached_tokens: u64,
}

#[derive(Deserialize)]
struct WireError {
    error: WireErrorBody,
}

#[derive(Deserialize)]
struct WireErrorBody {
    message: String,
}

impl ModelProvider for OpenRouter {
    fn complete(&self, request: CompletionRequest) -> CompletionFuture<'_> {
        Box::pin(self.post(request))
    }
}

impl OpenRouter {
    /// One POST to `chat/completions`, with the statuses mapped to the provider errors.
    async fn post(&self, request: CompletionRequest) -> Result<Completion, ProviderError> {
        let wire = WireRequest {
            model: &request.model,
            messages: request.messages.iter().map(|m| to_wire(m)).collect(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            tools: request.tools.iter().map(|t| to_wire_tool(t)).collect(),
        };
        let url = format!("{}/chat/completions", self.base_url);
        let mut post = self.client.post(&url).json(&wire);
        if !self.is_local() {
            post = post.bearer_auth(&self.key);
        }
        // the URL is in the error on purpose: a local server that is not running is the usual
        // cause, and the message says where the console looked
        let response = post.send().await.map_err(|e| ProviderError::Unavailable(format!("cannot reach {url}: {e}")))?;
        let status = response.status();
        let body = response.text().await.map_err(|e| ProviderError::Unavailable(format!("{url} dropped the answer: {e}")))?;
        match status {
            StatusCode::OK => {
                let parsed: WireResponse = serde_json::from_str(&body)
                    .map_err(|e| ProviderError::Other(format!("unparsable answer: {e}; body starts with: {}", snippet(&body))))?;
                let choice = parsed
                    .choices
                    .first()
                    .ok_or_else(|| ProviderError::Other(format!("the answer holds no choices; body starts with: {}", snippet(&body))))?;
                let tool_calls = choice.message.tool_calls();
                let text = choice.message.text();
                if tool_calls.is_empty() && text.trim().is_empty() {
                    return Err(ProviderError::Other(format!(
                        "the answer holds no text and no tool calls; body starts with: {}",
                        snippet(&body)
                    )));
                }
                let mut usage = parsed
                    .usage
                    .map(|u| Usage {
                        prompt_tokens: u.prompt_tokens,
                        completion_tokens: u.completion_tokens,
                        cached_tokens: u.prompt_tokens_details.map_or(0, |d| d.cached_tokens),
                        cost_usd: u.cost,
                        cost_computed: false,
                    })
                    .unwrap_or_default();
                if usage.cost_usd.is_none() {
                    self.price(&request.model, &mut usage).await;
                }
                Ok(Completion {
                    text,
                    usage,
                    turns: 1,
                    turn_usage: vec![crate::TurnUsage::of(&usage)],
                    tool_calls,
                    session_id: None,
                    gotchas: Vec::new(),
                    reads: Vec::new(),
                    read_in: 0,
                    whole_reads: 0,
                })
            }
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(ProviderError::Auth(error_text(&body, status))),
            StatusCode::NOT_FOUND => Err(ProviderError::UnknownModel(self.unknown_model_text(&body, status).await)),
            StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE | StatusCode::BAD_GATEWAY => {
                Err(ProviderError::Unavailable(error_text(&body, status)))
            }
            _ => Err(ProviderError::Other(error_text(&body, status))),
        }
    }
}

/// The message of the error body, or the status text when the body is not the usual shape.
fn error_text(body: &str, status: StatusCode) -> String {
    serde_json::from_str::<WireError>(body).ok().map(|e| e.error.message).unwrap_or_else(|| format!("HTTP {status}: {body}"))
}

/// The first 200 characters of a body, for an error message.
fn snippet(body: &str) -> String {
    let mut text: String = body.chars().take(200).collect();
    if body.chars().count() > 200 {
        text.push('…');
    }
    text
}

/// Maps one message of the conversation onto the wire. A tool message sends `content` even when it
/// is empty; an assistant message that asks for tool calls sends `content` only when it has text,
/// since the OpenRouter validation rejects an empty string next to tool calls.
fn to_wire(message: &Message) -> WireMessage<'_> {
    let empty_beside_calls = !message.tool_calls.is_empty() && message.content.trim().is_empty();
    WireMessage {
        role: &message.role,
        content: (!empty_beside_calls).then_some(message.content.as_str()),
        tool_calls: message.tool_calls.iter().map(|c| to_wire_call(c)).collect(),
        answering: message.tool_call_id.as_deref(),
    }
}

fn to_wire_call(call: &ToolCall) -> WireToolCall<'_> {
    WireToolCall { id: &call.id, kind: "function", function: WireFunction { name: &call.name, arguments: &call.arguments } }
}

fn to_wire_tool(spec: &ToolSpec) -> WireTool<'_> {
    WireTool {
        kind: "function",
        function: WireToolFunction { name: &spec.name, description: &spec.description, parameters: &spec.parameters },
    }
}

#[cfg(test)]
mod tests;
