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
            "stream": true,
            // Ask the provider for a usage block on the final chunk. Without it
            // a mission's token accounting is guesswork, and a budget that
            // cannot be measured cannot be enforced. Providers that do not know
            // the field ignore it.
            "stream_options": { "include_usage": true }
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
        // SSE frames arrive split at arbitrary network-chunk boundaries, so a
        // `data: {...}` line routinely straddles two chunks. Parsing each chunk
        // on its own silently dropped the first half (invalid JSON) and the
        // second half (no `data:` prefix) — losing text from replies with no
        // error anywhere. The carry buffer holds the unterminated tail until
        // the next chunk completes the line.
        let mut carry = String::new();
        while let Some(chunk) = stream.next().await {
            if cancel.load(Ordering::SeqCst) {
                break; // owner pressed stop — keep the partial answer
            }
            let chunk = chunk.map_err(|e| VaraError::Llm(format!("stream: {e}")))?;
            carry.push_str(&String::from_utf8_lossy(&chunk));
            raw.push_str(&String::from_utf8_lossy(&chunk));

            // Process only complete lines; keep the remainder for the next chunk.
            let mut lines: Vec<&str> = carry.split('\n').collect();
            let rest = lines.pop().unwrap_or("").to_string();
            for line in lines {
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
            carry = rest;
        }

        // A final line with no trailing newline is still a frame.
        if let Some(data) = carry.trim().strip_prefix("data:") {
            let data = data.trim();
            if data != "[DONE]" && !data.is_empty() {
                saw_sse = true;
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(d) = v["choices"][0]["delta"]["content"].as_str() {
                        if !d.is_empty() {
                            content.push_str(d);
                            on_delta(d);
                        }
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

#[cfg(test)]
mod tests {
    /// Reproduces the streaming bug: SSE frames arrive cut at arbitrary byte
    /// boundaries, so a `data:` line is routinely split across two network
    /// chunks. The old parser split each chunk on '\n' independently and dropped
    /// both halves silently; this feeds the same frame in three pieces and
    /// requires the text to survive.
    #[test]
    fn sse_frames_split_across_chunks_are_kept() {
        // What the provider sends, as one logical stream of bytes.
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\
                    data: {\"choices\":[{\"delta\":{\"content\":\"lo \"}}]}\n\
                    data: {\"choices\":[{\"delta\":{\"content\":\"world\"}}],\"model\":\"m\"}\n\
                    data: [DONE]\n";

        // Split it into chunks *inside* the frames — this is the failure mode.
        let bytes = body.as_bytes();
        let mut chunks: Vec<&[u8]> = Vec::new();
        let mut i = 0;
        let sizes = [17usize, 9, 23, 5, 11, 40, 60];
        for size in sizes {
            if i >= bytes.len() {
                break;
            }
            let end = (i + size).min(bytes.len());
            chunks.push(&bytes[i..end]);
            i = end;
        }

        let mut carry = String::new();
        let mut content = String::new();
        let mut model = String::new();
        for chunk in chunks {
            carry.push_str(&String::from_utf8_lossy(chunk));
            let mut lines: Vec<&str> = carry.split('\n').collect();
            let rest = lines.pop().unwrap_or("").to_string();
            for line in lines {
                let Some(data) = line.trim().strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim();
                if data == "[DONE]" || data.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(d) = v["choices"][0]["delta"]["content"].as_str() {
                        content.push_str(d);
                    }
                    if let Some(m) = v["model"].as_str() {
                        model = m.to_string();
                    }
                }
            }
            carry = rest;
        }
        // The tail frame (no trailing newline) must still be read.
        if let Some(data) = carry.trim().strip_prefix("data:") {
            let data = data.trim();
            if data != "[DONE]" && !data.is_empty() {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(d) = v["choices"][0]["delta"]["content"].as_str() {
                        content.push_str(d);
                    }
                }
            }
        }

        assert_eq!(
            content, "Hello world",
            "text was lost because frames were parsed per chunk"
        );
        assert_eq!(model, "m", "the model name from a split frame was lost");
    }

    /// The old behaviour, kept as the counter-example so the regression is
    /// explicit rather than implied: parsing each chunk alone loses the frame.
    #[test]
    fn parsing_each_chunk_alone_loses_the_frame() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n";
        let bytes = body.as_bytes();
        let a = &bytes[..17];
        let b = &bytes[17..];

        let mut content = String::new();
        for chunk in [a, b] {
            let piece = String::from_utf8_lossy(chunk);
            for line in piece.split('\n') {
                let Some(data) = line.trim().strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim();
                if data == "[DONE]" || data.is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(d) = v["choices"][0]["delta"]["content"].as_str() {
                        content.push_str(d);
                    }
                }
            }
        }
        assert!(
            content.is_empty(),
            "this is the bug being fixed: per-chunk parsing drops split frames"
        );
    }
}
