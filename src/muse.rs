use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;

#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Deserialize)]
struct Message {
    #[serde(default)]
    content: String,
}

#[derive(Deserialize)]
struct Transcript {
    #[serde(default)]
    transcript: String,
}

fn client() -> reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .expect("http client")
        })
        .clone()
}

pub fn transcribe(base_url: &str, key: &str, model: &str, wav: &[u8]) -> Result<String, String> {
    if key.is_empty() {
        return Err("set api_key in config.toml or MODEL_API_KEY".into());
    }
    let request = serde_json::json!({
        "mode": "PUSH_TO_TALK",
        "model": model,
        "audioEncoding": "WAV",
    })
    .to_string();
    let audio = reqwest::blocking::multipart::Part::bytes(wav.to_vec())
        .file_name("mic.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;
    let form = reqwest::blocking::multipart::Form::new()
        .part(
            "request",
            reqwest::blocking::multipart::Part::text(request)
                .mime_str("application/json")
                .map_err(|e| e.to_string())?,
        )
        .part("audio", audio);
    let res: Transcript = client()
        .post(format!("{base_url}/asr/transcribe"))
        .bearer_auth(key)
        .multipart(form)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;
    if res.transcript.trim().is_empty() {
        let path = std::env::temp_dir().join("trigger-last.wav");
        return Err(format!(
            "model heard nothing. Recording saved to {}",
            path.display()
        ));
    }
    Ok(res.transcript)
}

pub fn models(key: &str) -> Result<Vec<String>, String> {
    #[derive(Deserialize)]
    struct List {
        data: Vec<Model>,
    }
    #[derive(Deserialize)]
    struct Model {
        id: String,
    }
    let mut models: Vec<_> = client()
        .get(format!("{}/models", crate::config::MUSE_BASE_URL))
        .bearer_auth(key)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json::<List>()
        .map_err(|e| e.to_string())?
        .data
        .into_iter()
        .map(|m| m.id)
        .collect();
    models.sort();
    Ok(models)
}

fn chat_body(model: &str, prompt: &str, image_png: Option<&[u8]>) -> serde_json::Value {
    use base64::Engine;
    match image_png {
        Some(png) => {
            let data = base64::engine::general_purpose::STANDARD.encode(png);
            serde_json::json!({
                "model": model,
                "messages": [
                    {"role": "system", "content": "Answer the user's spoken question directly and concisely. Output only the answer. Use the screenshot as context when relevant."},
                    {"role": "user", "content": [
                        {"type": "text", "text": prompt},
                        {"type": "image_url", "image_url": {"url": format!("data:image/png;base64,{data}")}},
                    ]},
                ]
            })
        }
        None => serde_json::json!({
            "model": model,
            "messages": [
                {"role": "system", "content": "Answer the user's spoken question directly and concisely. Output only the answer."},
                {"role": "user", "content": prompt},
            ]
        }),
    }
}

pub fn ask(
    base_url: &str,
    key: &str,
    model: &str,
    prompt: &str,
    image_png: Option<&[u8]>,
) -> Result<String, String> {
    let body = chat_body(model, prompt, image_png);
    let res: ChatResponse = client()
        .post(format!("{base_url}/chat/completions"))
        .bearer_auth(key)
        .json(&body)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;
    res.choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .filter(|s| !s.trim().is_empty())
        .ok_or("empty reply from model".into())
}

#[cfg(test)]
mod tests {
    use super::chat_body;

    #[test]
    fn image_part_only_with_screenshot() {
        let plain = chat_body("m", "hi", None);
        assert_eq!(
            plain["messages"][1]["content"],
            serde_json::Value::String("hi".into())
        );
        let with = chat_body("m", "hi", Some(&[1, 2, 3]));
        let content = with["messages"][1]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert!(
            content[1]["image_url"]["url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
    }
}
