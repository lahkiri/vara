//! OpenAI-compatible chat client. Works with any provider that speaks the
//! standard protocol: Z.ai, OpenAI, Ollama, LM Studio, llama-server, vLLM…
//! One client, one protocol — the multi-model promise without glue code.

use crate::types::{ChatMessage, LlmReply, ProviderConfig};
use crate::{Result, VaraError};
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
