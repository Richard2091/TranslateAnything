//! OpenAI 兼容协议：POST `{base_url}/chat/completions`
//!
//! 批量翻译时把多条原文编号后一次发出（JSON 数组进、JSON 数组出），
//! 实时翻译框一屏几十段文字只需一个请求；模型没按格式返回时改为并发逐条翻译。

use super::{TranslateError, Translator, build_client, parse_err, send_json};
use crate::config::ProviderConfig;
use async_trait::async_trait;
use futures_util::future::join_all;
use serde_json::{Value, json};

/// 一次批量请求最多带多少条原文 / 多少字，超出的拆成多个请求并发发出
const BATCH_ITEMS: usize = 40;
const BATCH_CHARS: usize = 6000;
/// 退回逐条翻译时的并发数
const SINGLE_CONCURRENCY: usize = 4;

pub struct OpenAICompatible {
    client: reqwest::Client,
    url: String,
    api_key: String,
    model: String,
}

impl OpenAICompatible {
    pub fn new(cfg: &ProviderConfig) -> Result<Self, TranslateError> {
        let base = cfg.base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            return Err(TranslateError::Config(t!("err.engine.no_base_url")));
        }
        if cfg.model.trim().is_empty() {
            return Err(TranslateError::Config(t!("err.engine.no_model")));
        }
        Ok(Self {
            client: build_client(cfg)?,
            url: chat_url(base),
            api_key: cfg.api_key.trim().to_string(),
            model: cfg.model.trim().to_string(),
        })
    }

    /// 发一次对话请求，返回去掉思考过程后的回复正文
    async fn chat(&self, system: &str, user: &str) -> Result<String, TranslateError> {
        let mut req = self.client.post(&self.url).json(&json!({
            "model": self.model,
            "temperature": 0.2,
            "stream": false,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
        }));
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let v = send_json(req, "/error/message").await.map_err(|e| auth_hint(e, &self.url, &self.api_key))?;
        let msg = v.pointer("/choices/0/message").ok_or_else(|| parse_err(&v))?;
        let content = msg.get("content").and_then(Value::as_str).map(strip_think).unwrap_or_default();
        if content.is_empty() {
            // 思考模型把预算全花在思考上、正文为空（如 deepseek-reasoner 被截断），给出明确提示
            if msg.get("reasoning_content").and_then(Value::as_str).is_some_and(|s| !s.is_empty()) {
                return Err(TranslateError::Api(t!("err.engine.llm_only_reasoning")));
            }
            return Err(parse_err(&v));
        }
        Ok(content)
    }

    /// 一批原文一次请求；返回 None 表示模型没按格式返回
    async fn chat_batch(&self, texts: &[String], target_lang: &str) -> Result<Option<Vec<String>>, TranslateError> {
        let input = serde_json::to_string(texts).unwrap_or_default();
        let reply = self.chat(&batch_prompt(target_lang), &input).await?;
        Ok(parse_batch(&reply, texts.len()))
    }

    /// 逐条翻译（限制并发）
    async fn translate_each(&self, texts: &[String], target_lang: &str) -> Result<Vec<String>, TranslateError> {
        let mut out = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(SINGLE_CONCURRENCY) {
            for r in join_all(chunk.iter().map(|t| self.translate(t, target_lang))).await {
                out.push(r?);
            }
        }
        Ok(out)
    }
}

/// 对话接口地址：已经写到 `/chat/completions` 的原样使用
fn chat_url(base: &str) -> String {
    if base.ends_with("/chat/completions") { base.to_string() } else { format!("{base}/chat/completions") }
}

/// 鉴权失败（401 / 403）时补一句排查提示：多半是密钥和接口地址不是同一家服务商
fn auth_hint(e: TranslateError, url: &str, key: &str) -> TranslateError {
    match e {
        TranslateError::Api(m) if m.starts_with("HTTP 401") || m.starts_with("HTTP 403") => {
            let host = reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
            let mut m = format!("{m}\n{}", t!("err.engine.llm_auth_hint", host = host));
            // DeepSeek 官方密钥都以 sk- 开头；火山方舟、硅基流动等平台上的 DeepSeek 模型要填各自平台的地址
            if host.ends_with("deepseek.com") && !key.starts_with("sk-") {
                m.push('\n');
                m.push_str(&t!("err.engine.deepseek_key_hint"));
            }
            TranslateError::Api(m)
        }
        e => e,
    }
}

/// 去掉思考模型输出里的 `<think>…</think>`（部分接口把思考过程混在正文里返回）
fn strip_think(s: &str) -> String {
    let mut s = s.trim();
    if let Some(rest) = s.strip_prefix("<think>") {
        s = rest.split_once("</think>").map_or("", |(_, after)| after).trim();
    }
    s.to_string()
}

/// 解析批量回复：JSON 字符串数组（容忍 ```json 代码块、前后多余文字、`{"translations":[…]}` 包一层）
fn parse_batch(reply: &str, n: usize) -> Option<Vec<String>> {
    let start = reply.find(['[', '{'])?;
    let end = reply.rfind([']', '}'])?;
    let v: Value = serde_json::from_str(reply.get(start..=end)?).ok()?;
    let arr = match &v {
        Value::Array(a) => a,
        Value::Object(o) => o.values().find_map(Value::as_array)?,
        _ => return None,
    };
    let out: Vec<String> = arr.iter().map(|x| x.as_str().map(|s| s.trim().to_string())).collect::<Option<_>>()?;
    (out.len() == n).then_some(out)
}

/// 列出接口提供的模型。各家兼容接口基本都实现了 `GET /models`（返回 `data[].id`）；
/// Ollama 原生接口返回 `models[].name`，一并兼容
pub async fn list_models(cfg: &ProviderConfig) -> Result<Vec<String>, TranslateError> {
    let base = cfg.base_url.trim().trim_end_matches('/');
    let base = base.strip_suffix("/chat/completions").unwrap_or(base);
    if base.is_empty() {
        return Err(TranslateError::Config(t!("err.engine.no_base_url")));
    }
    let mut req = build_client(cfg)?.get(format!("{base}/models"));
    let key = cfg.api_key.trim();
    if !key.is_empty() {
        req = req.bearer_auth(key);
        // Anthropic 的模型列表只认自己的鉴权头
        if base.contains("anthropic.com") {
            req = req.header("x-api-key", key).header("anthropic-version", "2023-06-01");
        }
    }
    let v = send_json(req, "/error/message").await.map_err(|e| auth_hint(e, base, key))?;
    let list = v
        .pointer("/data")
        .or_else(|| v.pointer("/models"))
        .and_then(Value::as_array)
        .ok_or_else(|| parse_err(&v))?;
    // Gemini 的 id 带 "models/" 前缀，对话接口不需要
    let mut ids: Vec<String> = list
        .iter()
        .filter_map(|m| m.get("id").or_else(|| m.get("name")).and_then(Value::as_str))
        .map(|s| s.trim_start_matches("models/").to_string())
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

fn system_prompt(target_lang: &str) -> String {
    format!(
        "你是专业的翻译引擎。请把用户发来的文本翻译成目标语言（语言代码：{target_lang}）。\
         文本来自屏幕截图的 OCR 结果，可能存在被硬换行打断的句子和个别识别错误：\
         请按语义合并被打断的句子、保留原有的段落和列表结构、修正明显的识别错误。\
         如果文本本身已经是目标语言，原样输出。\
         只输出译文，不要任何解释、前后缀或引号。"
    )
}

fn batch_prompt(target_lang: &str) -> String {
    format!(
        "你是专业的翻译引擎。用户发来一个 JSON 字符串数组，每一项是屏幕上的一段文字（来自 OCR，可能有个别识别错误）。\
         请把每一项分别翻译成目标语言（语言代码：{target_lang}），修正明显的识别错误；已经是目标语言的项原样保留。\
         只输出一个 JSON 字符串数组：项数与输入完全相同、顺序一一对应，不要合并或拆分条目，不要任何解释或代码块标记。"
    )
}

#[async_trait]
impl Translator for OpenAICompatible {
    async fn translate(&self, text: &str, target_lang: &str) -> Result<String, TranslateError> {
        // 大模型自己能处理断行，直接传原始 OCR 文本
        self.chat(&system_prompt(target_lang), text).await
    }

    async fn translate_batch(&self, texts: &[String], target_lang: &str) -> Result<Vec<String>, TranslateError> {
        if texts.len() <= 1 {
            return self.translate_each(texts, target_lang).await;
        }
        // 按条数和字数分组，各组并发请求
        let mut groups: Vec<&[String]> = Vec::new();
        let (mut start, mut chars) = (0, 0);
        for (i, t) in texts.iter().enumerate() {
            let len = t.chars().count();
            if i > start && (i - start >= BATCH_ITEMS || chars + len > BATCH_CHARS) {
                groups.push(&texts[start..i]);
                (start, chars) = (i, 0);
            }
            chars += len;
        }
        groups.push(&texts[start..]);
        let results = join_all(groups.iter().map(|g| self.chat_batch(g, target_lang))).await;
        let mut out = Vec::with_capacity(texts.len());
        for (g, r) in groups.iter().zip(results) {
            match r? {
                Some(v) => out.extend(v),
                // 模型没按格式返回（条数不对、不是 JSON）：这一组改为逐条翻译
                None => out.extend(self.translate_each(g, target_lang).await?),
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 解析批量回复() {
        assert_eq!(parse_batch(r#"["你好","世界"]"#, 2), Some(vec!["你好".into(), "世界".into()]));
        assert_eq!(parse_batch("```json\n[\"a\", \"b\"]\n```", 2), Some(vec!["a".into(), "b".into()]));
        assert_eq!(parse_batch(r#"{"translations":["a"]}"#, 1), Some(vec!["a".into()]));
        assert_eq!(parse_batch(r#"["a"]"#, 2), None, "条数不符");
        assert_eq!(parse_batch("你好", 1), None);
    }

    #[test]
    fn 去掉思考过程() {
        assert_eq!(strip_think("<think>嗯……</think>\n你好"), "你好");
        assert_eq!(strip_think("  你好 "), "你好");
    }

    #[test]
    fn 对话地址() {
        assert_eq!(chat_url("https://api.deepseek.com"), "https://api.deepseek.com/chat/completions");
        assert_eq!(chat_url("https://x/v1/chat/completions"), "https://x/v1/chat/completions");
    }
}
