use std::{
    collections::HashSet,
    io::{BufRead, BufReader, Read as _},
    path::Path,
    time::Duration,
};

use anyhow::{Context as _, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::Deserialize;
use serde_json::Value;

use crate::settings::AiSettings;

pub fn chat_endpoint(base_url: &str) -> String {
    let base_url = base_url.trim().trim_end_matches('/');
    if base_url.ends_with("/chat/completions") {
        base_url.to_string()
    } else {
        format!("{base_url}/chat/completions")
    }
}

fn user_message(current_content: &str, prompt: &str) -> String {
    if current_content.trim().is_empty() {
        prompt.to_string()
    } else {
        format!(
            "Current document content:\n---\n{current_content}\n---\n\nWriting request:\n{prompt}"
        )
    }
}

pub fn stream_ai_text(
    provider: &AiSettings,
    current_content: &str,
    prompt: &str,
    mut on_delta: impl FnMut(&str),
) -> Result<()> {
    let body = serde_json::json!({
        "model": provider.model,
        "messages": [
            {
                "role": "system",
                "content": "You are a writing assistant inside a Markdown document editor. The existing document will be preserved and your response will be appended after it. Return only the new content to append, and do not repeat or rewrite the existing document. Do not add commentary or wrap the entire answer in code fences unless the user asks for code."
            },
            {
                "role": "user",
                "content": user_message(current_content, prompt)
            }
        ],
        "temperature": 0.7,
        "stream": true
    });
    let body = serde_json::to_string(&body)?;

    let mut request = ureq::post(&chat_endpoint(&provider.base_url))
        .set("Content-Type", "application/json")
        .set("Accept", "text/event-stream, application/json");
    if !provider.api_key.trim().is_empty() {
        request = request.set(
            "Authorization",
            &format!("Bearer {}", provider.api_key.trim()),
        );
    }

    let response = match request.send_string(&body) {
        Ok(response) => response,
        Err(ureq::Error::Status(status, response)) => {
            let text = response.into_string().unwrap_or_default();
            anyhow::bail!(
                "AI service returned {status}: {}",
                text.chars().take(600).collect::<String>()
            );
        }
        Err(error) => anyhow::bail!("AI request failed: {error}"),
    };

    let reader = BufReader::new(response.into_reader());
    let mut saw_stream_data = false;
    let mut plain_body = String::new();
    for line in reader.lines() {
        let line = line?;
        let Some(data) = line.strip_prefix("data:") else {
            plain_body.push_str(&line);
            plain_body.push('\n');
            continue;
        };
        saw_stream_data = true;
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let value: Value = serde_json::from_str(data)?;
        if let Some(delta) = value["choices"][0]["delta"]["content"].as_str() {
            on_delta(delta);
        }
    }

    if !saw_stream_data {
        let value: Value = serde_json::from_str(&plain_body)?;
        let content = value["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| {
                anyhow::anyhow!("AI response did not contain choices[0].message.content")
            })?;
        let characters = content.chars().collect::<Vec<_>>();
        for chunk in characters.chunks(16) {
            let text = chunk.iter().collect::<String>();
            on_delta(&text);
            std::thread::sleep(Duration::from_millis(18));
        }
    }

    Ok(())
}

pub fn image_endpoint(base_url: &str) -> String {
    let base_url = base_url.trim().trim_end_matches('/');
    let base_url = base_url
        .strip_suffix("/chat/completions")
        .unwrap_or(base_url);
    if base_url.ends_with("/images/generations") {
        base_url.to_string()
    } else {
        format!("{base_url}/images/generations")
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct AiPlanConnection {
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub to: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct AiPlanNode {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default)]
    pub node_type: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub svg: String,
    #[serde(default)]
    pub image_prompt: String,
    #[serde(skip)]
    pub image_path: Option<String>,
    #[serde(skip)]
    pub image_width: f32,
    #[serde(skip)]
    pub image_height: f32,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct AiBoardPlan {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub nodes: Vec<AiPlanNode>,
    #[serde(default)]
    pub connections: Vec<AiPlanConnection>,
    #[serde(skip)]
    pub image_failures: usize,
}

#[derive(Clone, Debug)]
pub enum AiBoardStage {
    Connecting,
    Planning,
    Parsing,
    GeneratingImage { index: usize, total: usize },
}

#[derive(Clone, Debug)]
pub enum AiBoardStreamEvent {
    Reasoning(String),
    Stage(AiBoardStage),
}

#[cfg(test)]
pub fn generate_board_plan(
    provider: &AiSettings,
    prompt: &str,
    include_images: bool,
    assets_root: &Path,
) -> Result<AiBoardPlan> {
    stream_board_plan(provider, prompt, include_images, assets_root, |_| {})
}

pub fn stream_board_plan(
    provider: &AiSettings,
    prompt: &str,
    include_images: bool,
    assets_root: &Path,
    mut on_event: impl FnMut(AiBoardStreamEvent),
) -> Result<AiBoardPlan> {
    on_event(AiBoardStreamEvent::Stage(AiBoardStage::Connecting));

    let system = r#"You are a whiteboard document planner inside a visual note-taking app.
Convert the user's request into a structured whiteboard document.
Return ONLY one valid JSON object, with no markdown fences and no commentary, using this schema:
{
  "title": "short document title",
  "nodes": [
    {"id": "n1", "type": "text", "title": "section title", "content": "Markdown content for one focused card"},
    {"id": "m1", "type": "mind", "title": "root topic", "parent": null},
    {"id": "m2", "type": "mind", "title": "branch", "parent": "m1"},
    {"id": "img1", "type": "image", "title": "short caption", "svg": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 800 500\">...</svg>", "image_prompt": "optional raster fallback description"}
  ],
  "connections": [
    {"from": "n1", "to": "n2"}
  ]
}
Rules:
- Split the document into 2 to 6 focused text cards. Each text card must be self-contained Markdown with a short heading.
- Use a mind map when the topic has a natural hierarchy. Mind nodes use parent ids, with null for roots.
- Include image nodes ONLY when a visual materially improves understanding, at most 2. Prefer a compact, valid, standalone SVG diagram in the svg field: include xmlns and viewBox (or width/height), use clear shapes, arrows, labels and a light background, and do not use scripts, foreignObject, external images, or web fonts. Only use image_prompt when a raster illustration is truly necessary.
- Connections may reference any ids that exist. Connect related text cards or connect a text card to a mind root.
- Keep titles short. Write the content in the same language as the user's request."#;

    let body = serde_json::json!({
        "model": provider.model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": prompt}
        ],
        "temperature": 0.5,
        "stream": true
    });
    let body = serde_json::to_string(&body)?;

    let mut request = ureq::post(&chat_endpoint(&provider.base_url))
        .timeout(Duration::from_secs(180))
        .set("Content-Type", "application/json")
        .set("Accept", "text/event-stream, application/json");
    if !provider.api_key.trim().is_empty() {
        request = request.set(
            "Authorization",
            &format!("Bearer {}", provider.api_key.trim()),
        );
    }

    on_event(AiBoardStreamEvent::Stage(AiBoardStage::Planning));
    let response = match request.send_string(&body) {
        Ok(response) => response,
        Err(ureq::Error::Status(status, response)) => {
            let text = response.into_string().unwrap_or_default();
            anyhow::bail!(
                "AI service returned {status}: {}",
                text.chars().take(600).collect::<String>()
            );
        }
        Err(error) => anyhow::bail!("AI request failed: {error}"),
    };

    let content = read_streamed_plan(response, &mut on_event)?;
    on_event(AiBoardStreamEvent::Stage(AiBoardStage::Parsing));

    let mut plan = parse_board_plan(&content)?;
    normalize_plan(&mut plan);
    if plan.nodes.is_empty() {
        anyhow::bail!("AI did not return any usable whiteboard nodes");
    }

    let raster_enabled = include_images && !provider.image_model.trim().is_empty();
    let raster_total = if raster_enabled {
        plan.nodes
            .iter()
            .filter(|node| {
                node.node_type == "image"
                    && clean_svg_code(&node.svg).is_none()
                    && !node.image_prompt.trim().is_empty()
            })
            .count()
    } else {
        0
    };
    let mut raster_index = 0usize;
    for node in plan
        .nodes
        .iter_mut()
        .filter(|node| node.node_type == "image")
    {
        if let Some(svg) = clean_svg_code(&node.svg) {
            match crate::model::save_generated_svg(assets_root, &svg) {
                Ok((path, width, height)) => {
                    node.image_path = Some(
                        path.strip_prefix(assets_root)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                    node.image_width = width;
                    node.image_height = height;
                }
                Err(_) => {
                    plan.image_failures += 1;
                }
            }
            continue;
        }
        let invalid_svg = !node.svg.trim().is_empty();
        if !raster_enabled || node.image_prompt.trim().is_empty() {
            if invalid_svg {
                plan.image_failures += 1;
            }
            continue;
        }
        raster_index += 1;
        on_event(AiBoardStreamEvent::Stage(AiBoardStage::GeneratingImage {
            index: raster_index,
            total: raster_total,
        }));
        match generate_image(provider, &node.image_prompt) {
            Ok(bytes) => match crate::model::save_generated_image(assets_root, &bytes) {
                Ok((path, width, height)) => {
                    node.image_path = Some(
                        path.strip_prefix(assets_root)
                            .unwrap_or(&path)
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                    node.image_width = width;
                    node.image_height = height;
                }
                Err(_) => plan.image_failures += 1,
            },
            Err(_) => plan.image_failures += 1,
        }
    }

    Ok(plan)
}

fn read_streamed_plan(
    response: ureq::Response,
    on_event: &mut impl FnMut(AiBoardStreamEvent),
) -> Result<String> {
    let reader = BufReader::new(response.into_reader());
    let mut content = String::new();
    let mut plain_body = String::new();
    let mut saw_stream = false;

    for line in reader.lines() {
        let line = line?;
        let Some(data) = line.strip_prefix("data:") else {
            plain_body.push_str(&line);
            plain_body.push('\n');
            continue;
        };
        saw_stream = true;
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        let choice = &value["choices"][0];
        let reasoning = choice["delta"]["reasoning_content"]
            .as_str()
            .or_else(|| choice["delta"]["reasoning"].as_str())
            .or_else(|| choice["delta"]["thinking"].as_str())
            .or_else(|| choice["message"]["reasoning_content"].as_str())
            .or_else(|| choice["message"]["reasoning"].as_str());
        if let Some(reasoning) = reasoning
            && !reasoning.is_empty()
        {
            on_event(AiBoardStreamEvent::Reasoning(reasoning.to_string()));
        }
        if let Some(text) = choice["delta"]["content"]
            .as_str()
            .or_else(|| choice["message"]["content"].as_str())
        {
            content.push_str(text);
        }
    }

    if !saw_stream {
        return extract_chat_content(&plain_body);
    }
    if content.trim().is_empty() {
        anyhow::bail!("AI stream did not contain document content");
    }
    Ok(content)
}

fn extract_chat_content(text: &str) -> Result<String> {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        if let Some(content) = value["choices"][0]["message"]["content"].as_str() {
            return Ok(content.to_string());
        }
        if let Some(content) = value["choices"][0]["delta"]["content"].as_str() {
            return Ok(content.to_string());
        }
    }

    let mut output = String::new();
    let mut saw_data = false;
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if let Some(delta) = value["choices"][0]["delta"]["content"].as_str() {
            saw_data = true;
            output.push_str(delta);
        }
    }
    if saw_data && !output.is_empty() {
        Ok(output)
    } else {
        anyhow::bail!(
            "AI response did not contain choices[0].message.content: {}",
            text.chars().take(300).collect::<String>()
        )
    }
}

fn parse_board_plan(content: &str) -> Result<AiBoardPlan> {
    let start = content
        .find('{')
        .ok_or_else(|| anyhow::anyhow!("AI response did not contain a JSON object"))?;
    let end = content
        .rfind('}')
        .filter(|end| *end >= start)
        .ok_or_else(|| anyhow::anyhow!("AI response did not contain a complete JSON object"))?;
    let value: Value = serde_json::from_str(&content[start..=end])
        .with_context(|| "AI response did not contain a valid board plan")?;
    serde_json::from_value(value).with_context(|| "AI board plan did not match the expected schema")
}

fn normalize_plan(plan: &mut AiBoardPlan) {
    let mut seen = HashSet::new();
    let mut generated_id = 0usize;
    for node in &mut plan.nodes {
        node.node_type = match node.node_type.trim().to_ascii_lowercase().as_str() {
            "mind" | "mindmap" | "mind_map" | "mind-map" => "mind".to_string(),
            "image" | "img" | "picture" => "image".to_string(),
            "text" | "card" | "note" => "text".to_string(),
            other => other.to_string(),
        };
        if node.id.trim().is_empty() {
            generated_id += 1;
            node.id = format!("n{generated_id}");
        }
        let base_id = node.id.trim().to_string();
        let mut id = base_id.clone();
        let mut suffix = 2usize;
        while !seen.insert(id.clone()) {
            id = format!("{base_id}-{suffix}");
            suffix += 1;
        }
        node.id = id;
        if node.node_type == "text" && node.content.trim().is_empty() {
            node.content = node.title.clone();
        }
        if node.node_type == "mind" && node.title.trim().is_empty() {
            node.title = node
                .content
                .lines()
                .next()
                .unwrap_or("Topic")
                .trim()
                .to_string();
        }
    }

    let mut text = 0usize;
    let mut mind = 0usize;
    let mut image = 0usize;
    plan.nodes.retain(|node| match node.node_type.as_str() {
        "text" => {
            text += 1;
            text <= 8
        }
        "mind" => {
            mind += 1;
            mind <= 30
        }
        "image" => {
            image += 1;
            image <= 2
        }
        _ => false,
    });
    plan.connections.retain(|connection| {
        !connection.from.trim().is_empty() && !connection.to.trim().is_empty()
    });
}

fn clean_svg_code(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let body = if let Some(rest) = trimmed.strip_prefix("```") {
        let start = rest.find('\n').map(|index| index + 1).unwrap_or(0);
        let end = rest.rfind("```").unwrap_or(rest.len());
        rest.get(start..end).unwrap_or("")
    } else {
        trimmed
    };
    let lower = body.to_ascii_lowercase();
    let start = lower.find("<svg")?;
    let end = lower.rfind("</svg>").map(|index| index + 6)?;
    let mut svg = body.get(start..end)?.trim().to_string();
    let lower = svg.to_ascii_lowercase();
    if [
        "<script",
        "<foreignobject",
        "javascript:",
        "onload=",
        "onclick=",
    ]
    .iter()
    .any(|blocked| lower.contains(blocked))
    {
        return None;
    }
    if !lower.contains("xmlns=") && svg.len() >= 4 {
        svg.insert_str(4, " xmlns=\"http://www.w3.org/2000/svg\"");
    }
    Some(svg)
}

fn generate_image(provider: &AiSettings, prompt: &str) -> Result<Vec<u8>> {
    let body = serde_json::json!({
        "model": provider.image_model,
        "prompt": prompt,
        "n": 1,
        "size": "1024x1024"
    });
    let body = serde_json::to_string(&body)?;

    let mut request = ureq::post(&image_endpoint(&provider.base_url))
        .timeout(Duration::from_secs(240))
        .set("Content-Type", "application/json")
        .set("Accept", "application/json");
    if !provider.api_key.trim().is_empty() {
        request = request.set(
            "Authorization",
            &format!("Bearer {}", provider.api_key.trim()),
        );
    }

    let response = match request.send_string(&body) {
        Ok(response) => response,
        Err(ureq::Error::Status(status, response)) => {
            let text = response.into_string().unwrap_or_default();
            anyhow::bail!(
                "Image service returned {status}: {}",
                text.chars().take(400).collect::<String>()
            );
        }
        Err(error) => anyhow::bail!("Image request failed: {error}"),
    };

    let text = response
        .into_string()
        .context("failed to read image response")?;
    let value: Value = serde_json::from_str(&text).context("image response was not valid JSON")?;
    if let Some(data) = value["data"][0]["b64_json"].as_str() {
        let data = data
            .strip_prefix("data:")
            .and_then(|data| data.split_once(','))
            .map(|(_, data)| data)
            .unwrap_or(data);
        return BASE64
            .decode(data)
            .context("image response contained invalid base64 data");
    }
    if let Some(url) = value["data"][0]["url"].as_str() {
        let response = ureq::get(url)
            .timeout(Duration::from_secs(240))
            .call()
            .context("failed to download generated image")?;
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(32 * 1024 * 1024)
            .read_to_end(&mut bytes)
            .context("failed to read generated image")?;
        if bytes.is_empty() {
            anyhow::bail!("generated image download was empty");
        }
        return Ok(bytes);
    }

    anyhow::bail!("image response did not contain data")
}

#[cfg(test)]
pub fn request_ai_text(
    provider: &AiSettings,
    current_content: &str,
    prompt: &str,
) -> Result<String> {
    let mut output = String::new();
    stream_ai_text(provider, current_content, prompt, |delta| {
        output.push_str(delta);
    })?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::net::{TcpListener, TcpStream};

    fn read_request(stream: &mut TcpStream) {
        let mut request = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            let read = stream.read(&mut buffer).unwrap_or(0);
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("Content-Length: ")
                            .or_else(|| line.strip_prefix("content-length: "))
                    })
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
        }
    }

    fn test_provider(base_url: String) -> AiSettings {
        AiSettings {
            base_url,
            api_key: String::new(),
            model: "test-model".to_string(),
            image_model: String::new(),
        }
    }

    #[test]
    fn endpoint_accepts_base_or_full_chat_url() {
        assert_eq!(
            chat_endpoint("https://api.openai.com/v1"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            chat_endpoint("https://example.com/chat/completions/"),
            "https://example.com/chat/completions"
        );
    }

    #[test]
    fn image_endpoint_accepts_base_or_full_url() {
        assert_eq!(
            image_endpoint("https://api.openai.com/v1"),
            "https://api.openai.com/v1/images/generations"
        );
        assert_eq!(
            image_endpoint("https://example.com/v1/images/generations/"),
            "https://example.com/v1/images/generations"
        );
        assert_eq!(
            image_endpoint("https://example.com/v1/chat/completions"),
            "https://example.com/v1/images/generations"
        );
    }

    #[test]
    fn parses_fenced_board_plan_and_normalizes_ids() {
        let content = r##"Here is the plan:
```json
{
  "title": "Plan",
  "nodes": [
    {"id": "n1", "type": "TEXT", "title": "Overview", "content": "# Overview"},
    {"id": "n1", "type": "mind", "title": "Root", "parent": null},
    {"type": "text", "title": "Fallback", "content": ""}
  ],
  "connections": [{"from": "n1", "to": "n1-2"}]
}
```"##;
        let mut plan = parse_board_plan(content).expect("fenced plan should parse");
        normalize_plan(&mut plan);
        assert_eq!(plan.title, "Plan");
        assert_eq!(plan.nodes.len(), 3);
        assert_eq!(plan.nodes[0].node_type, "text");
        assert_eq!(plan.nodes[1].node_type, "mind");
        assert!(plan.nodes[2].id.starts_with("n1-"));
        assert_eq!(plan.nodes[2].content, "Fallback");
        assert_eq!(plan.connections.len(), 1);
    }

    #[test]
    fn generates_a_structured_board_plan() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let address = listener.local_addr().expect("test server address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should arrive");
            read_request(&mut stream);
            let plan = serde_json::json!({
                "title": "Rust ownership",
                "nodes": [
                    {"id": "n1", "type": "text", "title": "Overview", "content": "# Overview\n\nRust uses ownership."},
                    {"id": "m1", "type": "mind", "title": "Ownership", "parent": null},
                    {"id": "m2", "type": "mind", "title": "Borrowing", "parent": "m1"}
                ],
                "connections": [{"from": "n1", "to": "m1"}]
            })
            .to_string();
            let content = format!("```json\n{plan}\n```");
            let body = serde_json::json!({
                "choices": [{"message": {"role": "assistant", "content": content}}]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response should be written");
        });

        let provider = test_provider(format!("http://{address}/v1"));
        let plan = generate_board_plan(&provider, "Explain Rust ownership", false, Path::new("."))
            .expect("board plan should be generated");
        assert_eq!(plan.title, "Rust ownership");
        assert_eq!(plan.nodes.len(), 3);
        assert_eq!(plan.connections.len(), 1);
        assert_eq!(plan.image_failures, 0);
        server.join().expect("test server should finish");
    }

    #[test]
    fn streams_reasoning_and_document_content() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let address = listener.local_addr().expect("test server address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should arrive");
            read_request(&mut stream);
            let plan = serde_json::json!({
                "title": "Streamed plan",
                "nodes": [
                    {"id": "n1", "type": "text", "title": "Overview", "content": "# Overview"},
                    {"id": "m1", "type": "mind", "title": "Root", "parent": null}
                ],
                "connections": [{"from": "n1", "to": "m1"}]
            })
            .to_string();
            let reasoning = serde_json::json!({
                "choices": [{"delta": {"reasoning_content": "First, split the topic. "}}]
            })
            .to_string();
            let content = serde_json::json!({
                "choices": [{"delta": {"content": plan}}]
            })
            .to_string();
            let body = format!("data: {reasoning}\n\ndata: {content}\n\ndata: [DONE]\n\n");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response should be written");
        });

        let provider = test_provider(format!("http://{address}/v1"));
        let mut reasoning = String::new();
        let mut stages = 0usize;
        let plan =
            stream_board_plan(
                &provider,
                "Explain",
                false,
                Path::new("."),
                |event| match event {
                    AiBoardStreamEvent::Reasoning(delta) => reasoning.push_str(&delta),
                    AiBoardStreamEvent::Stage(_) => stages += 1,
                },
            )
            .expect("streamed board plan should be generated");
        assert!(reasoning.contains("split the topic"));
        assert!(stages >= 3);
        assert_eq!(plan.title, "Streamed plan");
        assert_eq!(plan.nodes.len(), 2);
        assert_eq!(plan.connections.len(), 1);
        server.join().expect("test server should finish");
    }

    #[test]
    fn cleans_svg_fences_and_rejects_unsafe_markup() {
        let svg = clean_svg_code("```svg\n<svg><rect width=\"10\" height=\"10\"/></svg>\n```")
            .expect("fenced svg should be accepted");
        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(svg.contains("<rect"));
        assert!(clean_svg_code("<svg><script>alert(1)</script></svg>").is_none());
        assert!(clean_svg_code("<svg><foreignObject></foreignObject></svg>").is_none());
        assert!(clean_svg_code("no svg here").is_none());
    }

    #[test]
    fn generates_and_saves_svg_diagrams_without_an_image_model() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let address = listener.local_addr().expect("test server address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should arrive");
            read_request(&mut stream);
            let plan = serde_json::json!({
                "title": "SVG plan",
                "nodes": [
                    {"id": "n1", "type": "text", "title": "Overview", "content": "# Overview"},
                    {
                        "id": "img1",
                        "type": "image",
                        "title": "Diagram",
                        "svg": "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 800 500\"><rect width=\"800\" height=\"500\" fill=\"#eef2ff\"/><text x=\"40\" y=\"80\">Ownership</text></svg>"
                    }
                ],
                "connections": [{"from": "n1", "to": "img1"}]
            })
            .to_string();
            let body = serde_json::json!({
                "choices": [{"message": {"role": "assistant", "content": plan}}]
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response should be written");
        });

        let assets_root =
            std::env::temp_dir().join(format!("whiteboard-svg-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&assets_root).expect("temp assets folder");
        let provider = test_provider(format!("http://{address}/v1"));
        let plan = generate_board_plan(&provider, "Draw it", false, &assets_root)
            .expect("SVG plan should be generated");
        let image = plan
            .nodes
            .iter()
            .find(|node| node.node_type == "image")
            .expect("image node");
        let path = image.image_path.as_ref().expect("saved svg path");
        assert!(path.ends_with(".svg"));
        assert!(assets_root.join(path).exists());
        assert_eq!(image.image_width, 800.0);
        assert_eq!(image.image_height, 500.0);
        assert_eq!(plan.image_failures, 0);
        std::fs::remove_dir_all(&assets_root).ok();
        server.join().expect("test server should finish");
    }

    #[test]
    fn reads_an_openai_compatible_response() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let address = listener.local_addr().expect("test server address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should arrive");
            read_request(&mut stream);
            let body =
                r#"{"choices":[{"message":{"role":"assistant","content":"generated text"}}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response should be written");
        });

        let provider = test_provider(format!("http://{address}/v1"));
        let result = request_ai_text(&provider, "", "prompt").expect("AI request should succeed");
        assert_eq!(result, "generated text");
        server.join().expect("test server should finish");
    }

    #[test]
    fn emits_deltas_before_the_response_finishes() {
        use std::time::{Duration, Instant};

        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let address = listener.local_addr().expect("test server address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should arrive");
            read_request(&mut stream);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .expect("headers should be written");
            let write_chunk = |stream: &mut TcpStream, body: &str| {
                write!(stream, "{:x}\r\n{}\r\n", body.len(), body)
                    .expect("chunk should be written");
                stream.flush().expect("chunk should be flushed");
            };
            write_chunk(
                &mut stream,
                "data: {\"choices\":[{\"delta\":{\"content\":\"first\"}}]}\n\n",
            );
            std::thread::sleep(Duration::from_millis(300));
            write_chunk(
                &mut stream,
                "data: {\"choices\":[{\"delta\":{\"content\":\"second\"}}]}\n\n",
            );
            write_chunk(&mut stream, "data: [DONE]\n\n");
            stream
                .write_all(b"0\r\n\r\n")
                .expect("terminating chunk should be written");
        });

        let provider = test_provider(format!("http://{address}/v1"));
        let started = Instant::now();
        let mut arrivals = Vec::new();
        stream_ai_text(&provider, "", "prompt", |delta| {
            arrivals.push((started.elapsed(), delta.to_string()));
        })
        .expect("AI request should succeed");
        assert_eq!(arrivals.len(), 2);
        assert_eq!(arrivals[0].1, "first");
        assert!(arrivals[0].0 < Duration::from_millis(200));
        assert_eq!(arrivals[1].1, "second");
        server.join().expect("test server should finish");
    }

    #[test]
    fn reads_streaming_sse_deltas() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let address = listener.local_addr().expect("test server address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should arrive");
            read_request(&mut stream);
            let body = concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"hello \"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"world\"}}]}\n\n",
                "data: [DONE]\n\n"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response should be written");
        });

        let provider = test_provider(format!("http://{address}/v1"));
        let result = request_ai_text(&provider, "", "prompt").expect("AI request should succeed");
        assert_eq!(result, "hello world");
        server.join().expect("test server should finish");
    }
}
