//! OpenAI-compatible chat client. Works with any provider that speaks the
//! standard protocol: Z.ai, OpenAI, Ollama, LM Studio, llama-server, vLLM…
//! One client, one protocol — the multi-model promise without glue code.

use crate::types::{ChatMessage, LlmReply, ProviderConfig};
use crate::{Result, VaraError};
use futures_util::StreamExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub struct LlmClient {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    temperature: f32,
}

impl LlmClient {
    pub fn new(cfg: &ProviderConfig) -> Result<Self> {
        let base_url = cfg.base_url.trim().trim_end_matches('/').to_string();
        if base_url.is_empty() {
            return Err(VaraError::Llm(
                "provider base_url is empty — open Settings and configure a model provider".into(),
            ));
        }
        if cfg.model.trim().is_empty() {
            return Err(VaraError::Llm(
                "model name is empty — set it in Settings".into(),
            ));
        }
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(240))
            .build()
            .map_err(|e| VaraError::Llm(e.to_string()))?;
        Ok(Self {
            http,
            base_url,
            api_key: cfg.api_key.clone(),
            model: cfg.model.trim().to_string(),
            temperature: cfg.temperature.clamp(0.0, 2.0),
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub async fn chat(
        &self,
        messages: &[ChatMessage],
        max_tokens: Option<u32>,
    ) -> Result<LlmReply> {
        let url = format!("{}/chat/completions", self.base_url);
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "temperature": self.temperature,
            "stream": false
        });
        if let Some(mt) = max_tokens {
            body["max_tokens"] = serde_json::json!(mt);
        }
        let mut req = self.http.post(&url).json(&body);
        let key = self.api_key.trim();
        if !key.is_empty() {
            req = req.bearer_auth(key);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| VaraError::Llm(format!("request failed: {e}")))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            let snippet: String = text.chars().take(300).collect();
            return Err(VaraError::Llm(format!("HTTP {status}: {snippet}")));
        }
        let v: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| VaraError::Llm(format!("bad JSON from provider: {e}")))?;
        let content = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_string();
        if content.is_empty() {
            let snippet = serde_json::to_string(&v).unwrap_or_default();
            let snippet: String = snippet.chars().take(300).collect();
            return Err(VaraError::Llm(format!(
                "empty completion from provider: {snippet}"
            )));
        }
        let model = v["model"].as_str().unwrap_or(&self.model).to_string();
        let prompt_tokens = v["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
        let completion_tokens = v["usage"]["completion_tokens"].as_u64().unwrap_or(0);
        Ok(LlmReply {
            content,
            model,
            prompt_tokens,
            completion_tokens,
        })
    }

    /// Small ping used by the Settings "Test connection" button.
    pub async fn health_check(&self) -> Result<(String, u128)> {
        let t0 = std::time::Instant::now();
        let reply = self
            .chat(&[ChatMessage::user("Reply with exactly: OK")], Some(8))
            .await?;
        Ok((reply.content.trim().to_string(), t0.elapsed().as_millis()))
    }

    /// Streaming chat completion (SSE). Every text chunk is handed to
    /// `on_delta` the moment it arrives; the aggregate is returned at the end.
    /// Providers that ignore `stream:true` and answer with one JSON body are
    /// handled by the fallback parser, so the UI never breaks either way.
    /// `cancel` aborts mid-stream — whatever was generated so far is kept.
    pub async fn chat_stream(
        &self,
        messages: &[ChatMessage],
        max_tokens: Option<u32>,
        cancel: Arc<AtomicBool>,
        mut on_delta: impl FnMut(&str),
    ) -> Result<LlmReply> {
        let url = format!("{}/chat/completions", self.base_url);
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "temperature": self.temperature,
            "stream": true
        });
        if let Some(mt) = max_tokens {
            body["max_tokens"] = serde_json::json!(mt);
        }
        let mut req = self
            .http
            .post(&url)
            .json(&body)
            .timeout(Duration::from_secs(600));
        let key = self.api_key.trim();
        if !key.is_empty() {
            req = req.bearer_auth(key);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| VaraError::Llm(format!("request failed: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let snippet: String = text.chars().take(300).collect();
            return Err(VaraError::Llm(format!("HTTP {status}: {snippet}")));
        }

        let mut raw = String::new();
        let mut content = String::new();
        let mut model = self.model.clone();
        let (mut ptok, mut ctok) = (0u64, 0u64);
        let mut saw_sse = false;

        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            if cancel.load(Ordering::SeqCst) {
                break; // owner pressed stop — keep the partial answer
            }
            let chunk = chunk.map_err(|e| VaraError::Llm(format!("stream: {e}")))?;
            let piece = String::from_utf8_lossy(&chunk);
            raw.push_str(&piece);
            for line in piece.split('\n') {
                let Some(data) = line.trim().strip_prefix("data:") else {
                    continue;
                };
                saw_sse = true;
                let data = data.trim();
                if data == "[DONE]" || data.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(err) = v["error"]["message"].as_str() {
                        return Err(VaraError::Llm(err.to_string()));
                    }
                    if let Some(d) = v["choices"][0]["delta"]["content"].as_str() {
                        if !d.is_empty() {
                            content.push_str(d);
                            on_delta(d);
                        }
                    }
                    if let Some(m) = v["model"].as_str() {
                        model = m.to_string();
                    }
                    if let Some(p) = v["usage"]["prompt_tokens"].as_u64() {
                        ptok = p;
                    }
                    if let Some(c) = v["usage"]["completion_tokens"].as_u64() {
                        ctok = c;
                    }
                }
            }
        }

        // Fallback: provider answered with one plain JSON body instead of SSE.
        if !saw_sse && content.is_empty() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw.trim()) {
                if let Some(c) = v["choices"][0]["message"]["content"].as_str() {
                    content = c.trim().to_string();
                    if !content.is_empty() {
                        on_delta(&content);
                    }
                }
                if let Some(m) = v["model"].as_str() {
                    model = m.to_string();
                }
                ptok = v["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
                ctok = v["usage"]["completion_tokens"].as_u64().unwrap_or(0);
            }
        }

        if content.is_empty() {
            return Err(VaraError::Llm(
                "empty completion from provider (no streamed content)".into(),
            ));
        }
        let model_name = model;
        Ok(LlmReply {
            content,
            model: model_name,
            prompt_tokens: ptok,
            completion_tokens: ctok,
        })
    }
}

/// Extract the first JSON object from a model reply (models love to add prose).
pub fn extract_json(s: &str) -> Option<serde_json::Value> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&s[start..=end]).ok()
}
