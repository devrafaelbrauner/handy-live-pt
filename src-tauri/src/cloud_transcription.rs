//! Cloud speech-to-text for finished recordings.
//!
//! Every supported provider (OpenAI, OpenRouter, Groq, or any compatible
//! server) accepts the OpenAI `POST {base_url}/audio/transcriptions` multipart
//! upload, so one request shape covers them all. The recording is encoded in
//! memory as 16 kHz mono 16-bit PCM WAV, the format Handy already records.
//! Long recordings are split at the quietest moment before each
//! [`MAX_CHUNK_SECS`] boundary, keeping every upload far below the 25 MB cap
//! these APIs share, and the parts are transcribed in order.
//!
//! API keys come from `AppSettings::post_process_api_keys`, shared with
//! post-processing so a key is entered once per provider.

use crate::llm_client::{report_reqwest_error, sanitized_url_for_log};
use crate::settings::{
    AppSettings, CloudTranscriptionProvider, CLOUD_TRANSCRIPTION_CUSTOM_PROVIDER_ID,
};
use log::{debug, info};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, REFERER, USER_AGENT};
use reqwest::multipart::{Form, Part};
use reqwest::StatusCode;
use serde::Deserialize;
use std::io::Cursor;
use std::ops::Range;
use std::time::{Duration, Instant};

const SAMPLE_RATE: usize = 16_000;
/// Longest audio sent in one request. Five minutes of 16 kHz mono 16-bit WAV
/// is ~9.6 MB, well under the 25 MB upload cap, and short enough to finish
/// inside the 60 s upstream timeout OpenRouter documents.
const MAX_CHUNK_SECS: usize = 300;
/// How far before a chunk boundary to look for a quiet place to split.
const SPLIT_SEARCH_SECS: usize = 3;
/// Window used to measure loudness while looking for a split point (20 ms).
const SPLIT_FRAME_SAMPLES: usize = SAMPLE_RATE / 50;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(180);
const MODEL_LIST_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest provider error message passed on to the user.
const MAX_ERROR_DETAIL_CHARS: usize = 300;

#[derive(Debug, Deserialize)]
struct TranscriptionResponse {
    text: String,
    /// Languages `gpt-transcribe` detected in the audio; absent elsewhere.
    #[serde(default)]
    languages: Option<Vec<DetectedLanguage>>,
}

#[derive(Debug, Deserialize)]
struct DetectedLanguage {
    code: String,
}

/// Everything one cloud transcription needs, resolved from settings up front
/// so configuration problems surface before any audio is uploaded.
struct CloudRequest<'a> {
    provider: &'a CloudTranscriptionProvider,
    api_key: &'a str,
    model: &'a str,
    language: Option<String>,
}

/// Transcribe a finished 16 kHz mono recording with the cloud provider chosen
/// in settings. Returns the text after Handy's usual cleanup (custom-word
/// correction, filler removal); error messages are meant for the user.
pub async fn transcribe(settings: &AppSettings, audio: &[f32]) -> Result<String, String> {
    if audio.is_empty() {
        return Ok(String::new());
    }

    let request = resolve_request(settings)?;
    let label = request.provider.label.as_str();
    let (fields, custom_words_prompted) = upload_fields(
        &request.provider.id,
        request.model,
        request.language.as_deref(),
        &settings.custom_words,
    );
    let url = format!(
        "{}/audio/transcriptions",
        request.provider.base_url.trim().trim_end_matches('/')
    );
    let client = build_client(request.api_key, REQUEST_TIMEOUT)?;
    let parts = chunk_ranges(
        audio,
        MAX_CHUNK_SECS * SAMPLE_RATE,
        SPLIT_SEARCH_SECS * SAMPLE_RATE,
        SPLIT_FRAME_SAMPLES,
    );

    info!(
        "Cloud transcription via {} (model '{}', language {:?}) at {}: {:.2}s of audio in {} request(s)",
        label,
        request.model,
        request.language,
        sanitized_url_for_log(&url),
        audio.len() as f64 / SAMPLE_RATE as f64,
        parts.len()
    );

    let started = Instant::now();
    let mut texts = Vec::with_capacity(parts.len());
    let mut detected_language = None;
    for part in parts {
        let wav = encode_wav(&audio[part])?;
        let response = transcribe_part(&client, &url, label, request.model, &fields, wav).await?;
        if detected_language.is_none() {
            detected_language = response
                .languages
                .and_then(|languages| languages.into_iter().next())
                .map(|language| language.code);
        }
        let text = response.text.trim();
        if !text.is_empty() {
            texts.push(text.to_string());
        }
    }
    info!(
        "Cloud transcription via {} finished in {:.2}s",
        label,
        started.elapsed().as_secs_f64()
    );

    Ok(crate::managers::transcription::clean_cloud_transcription(
        texts.join(" "),
        settings,
        custom_words_prompted,
        request.language.as_deref(),
        detected_language,
    ))
}

/// List the speech-to-text models a provider offers, for the model picker.
pub async fn fetch_models(
    provider: &CloudTranscriptionProvider,
    api_key: &str,
) -> Result<Vec<String>, String> {
    let base_url = provider.base_url.trim().trim_end_matches('/');
    let url = if provider.id == "openrouter" {
        // OpenRouter's catalogue is mostly chat models; ask for STT only.
        format!("{base_url}/models?output_modalities=transcription")
    } else {
        format!("{base_url}/models")
    };
    debug!(
        "Fetching cloud transcription models from {}",
        sanitized_url_for_log(&url)
    );

    let client = build_client(api_key, MODEL_LIST_TIMEOUT)?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| describe_transport_error(&provider.label, &error))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(describe_api_error(&provider.label, None, status, &body));
    }

    let listing: serde_json::Value = response.json().await.map_err(|error| {
        report_reqwest_error("Failed to parse cloud model list", &error);
        format!(
            "{} returned a model list Handy could not read.",
            provider.label
        )
    })?;
    Ok(transcription_model_ids(&provider.id, &listing))
}

/// The API key stored for a provider id, shared with post-processing.
pub(crate) fn api_key_for<'a>(settings: &'a AppSettings, provider_id: &str) -> &'a str {
    settings
        .post_process_api_keys
        .get(provider_id)
        .map(|key| key.trim())
        .unwrap_or_default()
}

fn resolve_request(settings: &AppSettings) -> Result<CloudRequest<'_>, String> {
    let provider = settings
        .cloud_transcription_provider(&settings.cloud_transcription_provider_id)
        .ok_or_else(|| {
            format!(
                "Cloud transcription provider '{}' is not available.",
                settings.cloud_transcription_provider_id
            )
        })?;

    let api_key = api_key_for(settings, &provider.id);
    // A self-hosted server may run without authentication.
    if api_key.is_empty() && provider.id != CLOUD_TRANSCRIPTION_CUSTOM_PROVIDER_ID {
        return Err(format!(
            "Add your {} API key in Settings → Cloud API to use cloud transcription.",
            provider.label
        ));
    }

    let model = settings
        .cloud_transcription_models
        .get(&provider.id)
        .map(|model| model.trim())
        .unwrap_or_default();
    if model.is_empty() {
        return Err(format!(
            "Choose a {} transcription model in Settings → Cloud API.",
            provider.label
        ));
    }

    if provider.base_url.trim().is_empty() {
        return Err(format!(
            "Set the base URL for {} in Settings → Cloud API.",
            provider.label
        ));
    }

    Ok(CloudRequest {
        provider,
        api_key,
        model,
        language: language_hint(&settings.selected_language),
    })
}

/// The ISO-639-1 code these APIs accept for Handy's language intent, or `None`
/// to let the provider detect it. Script variants collapse to their language
/// (`zh-Hant` → `zh`); codes without a two-letter form (e.g. `yue`) fall back
/// to detection instead of failing the request.
fn language_hint(selected_language: &str) -> Option<String> {
    let primary = selected_language
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    (primary.len() == 2 && primary.chars().all(|c| c.is_ascii_alphabetic())).then_some(primary)
}

/// Multipart text fields for one upload, plus whether the custom words were
/// handed to the provider (local fuzzy correction is then skipped, as it is
/// for whisper's initial prompt).
fn upload_fields(
    provider_id: &str,
    model: &str,
    language: Option<&str>,
    custom_words: &[String],
) -> (Vec<(&'static str, String)>, bool) {
    let mut fields = vec![("model", model.to_string())];
    let words: Vec<&str> = custom_words
        .iter()
        .map(|word| word.trim())
        .filter(|word| !word.is_empty())
        .collect();

    if provider_id == "openrouter" {
        // OpenRouter normalizes `language` across its STT providers and
        // ignores `prompt`, so custom words stay with local correction.
        if let Some(language) = language {
            fields.push(("language", language.to_string()));
        }
        return (fields, false);
    }

    if uses_gpt_transcribe_fields(model) {
        // gpt-transcribe replaces `language` with `languages[]` and takes
        // vocabulary as `keywords[]`. The API rejects the whole request if a
        // keyword contains `<`, `>` or a line break.
        if let Some(language) = language {
            fields.push(("languages[]", language.to_string()));
        }
        let keywords: Vec<&str> = words
            .into_iter()
            .filter(|word| !word.contains(['<', '>', '\r', '\n']))
            .collect();
        let prompted = !keywords.is_empty();
        fields.extend(
            keywords
                .into_iter()
                .map(|keyword| ("keywords[]", keyword.to_string())),
        );
        return (fields, prompted);
    }

    if let Some(language) = language {
        fields.push(("language", language.to_string()));
    }
    let prompted = !words.is_empty();
    if prompted {
        fields.push(("prompt", words.join(", ")));
    }
    (fields, prompted)
}

/// `gpt-transcribe` and its snapshots take `languages[]`/`keywords[]` instead
/// of `language`/`prompt`. Gateways may prefix the vendor (`openai/…`).
fn uses_gpt_transcribe_fields(model: &str) -> bool {
    let name = model.rsplit('/').next().unwrap_or(model);
    name == "gpt-transcribe" || name.starts_with("gpt-transcribe-")
}

fn build_client(api_key: &str, timeout: Duration) -> Result<reqwest::Client, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static("Handy/1.0 (+https://github.com/cjpais/Handy)"),
    );
    // OpenRouter attributes requests to an app through these two headers.
    headers.insert(
        REFERER,
        HeaderValue::from_static("https://github.com/cjpais/Handy"),
    );
    headers.insert("X-Title", HeaderValue::from_static("Handy"));
    if !api_key.is_empty() {
        let mut authorization = HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|_| "The API key contains characters that are not allowed.".to_string())?;
        authorization.set_sensitive(true);
        headers.insert(AUTHORIZATION, authorization);
    }

    reqwest::Client::builder()
        .default_headers(headers)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(timeout)
        .build()
        .map_err(|error| report_reqwest_error("Failed to build HTTP client", &error))
}

async fn transcribe_part(
    client: &reqwest::Client,
    url: &str,
    label: &str,
    model: &str,
    fields: &[(&'static str, String)],
    wav: Vec<u8>,
) -> Result<TranscriptionResponse, String> {
    let file = Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|error| report_reqwest_error("Failed to attach audio", &error))?;
    let form = fields
        .iter()
        .fold(Form::new(), |form, (name, value)| {
            form.text(*name, value.clone())
        })
        .part("file", file);

    let response = client
        .post(url)
        .multipart(form)
        .send()
        .await
        .map_err(|error| describe_transport_error(label, &error))?;
    let status = response.status();
    debug!(
        "Cloud transcription response from {}: {} over {:?}",
        label,
        status,
        response.version()
    );
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(describe_api_error(label, Some(model), status, &body));
    }

    response.json().await.map_err(|error| {
        // The decode error is logged without its source: it could quote the
        // transcript.
        report_reqwest_error("Failed to parse cloud transcription response", &error);
        format!("{label} returned a response Handy could not read.")
    })
}

fn describe_transport_error(label: &str, error: &reqwest::Error) -> String {
    let details = report_reqwest_error("Cloud transcription request failed", error);
    if error.is_timeout() {
        format!("{label} did not answer in time. Try again in a moment.")
    } else if error.is_connect() {
        format!("Could not connect to {label}. Check your internet connection and the provider settings.")
    } else {
        format!("The request to {label} failed: {details}")
    }
}

/// A user-facing explanation for a failed request. `model` is `None` for the
/// model listing. Rejected-key answers never echo the provider's message,
/// since some quote part of the key back.
fn describe_api_error(label: &str, model: Option<&str>, status: StatusCode, body: &str) -> String {
    let code = status.as_u16();
    let summary = match code {
        401 | 403 => {
            return format!(
                "{label} rejected the API key (HTTP {code}). Check the key in Settings → Cloud API."
            )
        }
        402 => format!("{label} reports insufficient credits (HTTP 402)."),
        404 => match model {
            Some(model) => format!(
                "{label} could not find the model '{model}' or the transcription endpoint (HTTP 404)."
            ),
            None => format!("{label} could not find the model list (HTTP 404)."),
        },
        413 => format!("The recording is too large for {label} (HTTP 413)."),
        429 => format!("{label} rate limit or quota exceeded (HTTP 429)."),
        _ => match model {
            Some(_) => format!("{label} could not transcribe the recording (HTTP {code})."),
            None => format!("{label} could not list its models (HTTP {code})."),
        },
    };
    match provider_error_message(body) {
        Some(detail) => format!("{summary} {detail}"),
        None => summary,
    }
}

/// The provider's own error message, when the body carries one, shortened to
/// fit a notification.
fn provider_error_message(body: &str) -> Option<String> {
    let parsed = serde_json::from_str::<serde_json::Value>(body).ok();
    let message = match &parsed {
        Some(json) => ["/error/message", "/error", "/message", "/detail"]
            .iter()
            .find_map(|pointer| json.pointer(pointer).and_then(|value| value.as_str()))
            .map(str::to_string),
        // Plain-text bodies are kept; HTML error pages are noise.
        None => Some(body.to_string()).filter(|text| !text.trim_start().starts_with('<')),
    }?;
    let message = message.trim();
    if message.is_empty() {
        return None;
    }
    let mut shortened: String = message.chars().take(MAX_ERROR_DETAIL_CHARS).collect();
    if shortened.len() < message.len() {
        shortened.push('…');
    }
    Some(shortened)
}

/// Model ids from an OpenAI-style `{ "data": [{ "id": … }] }` listing. OpenAI
/// and Groq list chat models too, so only speech-to-text ids are kept for them;
/// OpenRouter filters server-side and a custom server lists what it serves.
fn transcription_model_ids(provider_id: &str, listing: &serde_json::Value) -> Vec<String> {
    let speech_only = matches!(provider_id, "openai" | "groq");
    let mut ids: Vec<String> = listing
        .get("data")
        .and_then(|data| data.as_array())
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("id").and_then(|id| id.as_str()))
        .filter(|id| !speech_only || is_speech_to_text_model(id))
        .map(str::to_string)
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

fn is_speech_to_text_model(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    id.contains("transcribe") || id.contains("whisper")
}

fn encode_wav(samples: &[f32]) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let encode_error = |error: hound::Error| format!("Failed to encode the recording: {error}");
    let sample_count = u32::try_from(samples.len())
        .map_err(|_| "The recording is too long to encode.".to_string())?;

    let mut buffer = Cursor::new(Vec::with_capacity(44 + samples.len() * 2));
    let mut writer = hound::WavWriter::new(&mut buffer, spec).map_err(encode_error)?;
    let mut sample_writer = writer.get_i16_writer(sample_count);
    for &sample in samples {
        sample_writer.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
    }
    sample_writer.flush().map_err(encode_error)?;
    writer.finalize().map_err(encode_error)?;
    Ok(buffer.into_inner())
}

/// Split `samples` into consecutive ranges of at most `max_len` samples. Each
/// cut lands in the middle of the quietest `frame`-long window within `search`
/// samples before the limit, so words are not sliced in half.
fn chunk_ranges(samples: &[f32], max_len: usize, search: usize, frame: usize) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    while samples.len() - start > max_len {
        let limit = start + max_len;
        let window_start = limit.saturating_sub(search).max(start + frame);
        let cut = quietest_point(samples, window_start, limit, frame);
        ranges.push(start..cut);
        start = cut;
    }
    ranges.push(start..samples.len());
    ranges
}

/// Midpoint of the lowest-energy `frame`-long window in `from..to`, or `to`
/// when the span holds no full window.
fn quietest_point(samples: &[f32], from: usize, to: usize, frame: usize) -> usize {
    let mut quietest = to;
    let mut lowest_energy = f32::INFINITY;
    let mut window = from;
    while window + frame <= to {
        let energy: f32 = samples[window..window + frame]
            .iter()
            .map(|sample| sample * sample)
            .sum();
        if energy < lowest_energy {
            lowest_energy = energy;
            quietest = window + frame / 2;
        }
        window += frame;
    }
    quietest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::get_default_settings;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A captured HTTP request: lower-cased header lines and the raw body.
    struct CapturedRequest {
        head: String,
        body: Vec<u8>,
    }

    impl CapturedRequest {
        fn body_text(&self) -> String {
            String::from_utf8_lossy(&self.body).into_owned()
        }

        /// Values of a multipart text field, in the order they were sent.
        fn field(&self, name: &str) -> Vec<String> {
            let body = self.body_text();
            let marker = format!("name=\"{name}\"\r\n\r\n");
            body.match_indices(&marker)
                .map(|(index, _)| {
                    let value = &body[index + marker.len()..];
                    value[..value.find("\r\n").unwrap()].to_string()
                })
                .collect()
        }
    }

    /// Serve exactly one HTTP request on a random local port, answering with
    /// `status` and `body`. Returns the base URL and a handle yielding the
    /// request the client sent.
    async fn serve_once(
        status: &'static str,
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<CapturedRequest>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut buffer = [0_u8; 8192];
            let (head, body_start, content_length) = loop {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0, "client closed before sending headers");
                raw.extend_from_slice(&buffer[..read]);
                if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&raw[..end]).to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .map(|value| value.trim().parse::<usize>().unwrap())
                        .expect("multipart uploads carry a content-length");
                    break (head, end + 4, length);
                }
            };
            while raw.len() < body_start + content_length {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0, "client closed before sending the body");
                raw.extend_from_slice(&buffer[..read]);
            }
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            CapturedRequest {
                head,
                body: raw[body_start..body_start + content_length].to_vec(),
            }
        });
        (format!("http://{address}/v1"), handle)
    }

    fn custom_endpoint_settings(base_url: &str) -> AppSettings {
        let mut settings = get_default_settings();
        settings.cloud_transcription_enabled = true;
        settings.cloud_transcription_provider_id = CLOUD_TRANSCRIPTION_CUSTOM_PROVIDER_ID.into();
        settings
            .cloud_transcription_provider_mut(CLOUD_TRANSCRIPTION_CUSTOM_PROVIDER_ID)
            .unwrap()
            .base_url = base_url.to_string();
        settings.post_process_api_keys.insert(
            CLOUD_TRANSCRIPTION_CUSTOM_PROVIDER_ID.into(),
            "test-key".into(),
        );
        settings
    }

    fn tone(seconds: f32) -> Vec<f32> {
        (0..(seconds * SAMPLE_RATE as f32) as usize)
            .map(|i| (i as f32 * 0.05).sin() * 0.5)
            .collect()
    }

    #[tokio::test]
    async fn uploads_wav_with_fields_and_cleans_the_text() {
        let (base_url, server) = serve_once("200 OK", r#"{"text":"  Olá, uh   mundo  "}"#).await;
        let mut settings = custom_endpoint_settings(&base_url);
        settings.selected_language = "pt".into();
        settings.custom_words = vec!["Handy".into(), "  ".into(), "STF".into()];
        let audio = tone(1.0);

        let text = transcribe(&settings, &audio).await.unwrap();
        let request = server.await.unwrap();

        // Filler removal and whitespace cleanup ran on the provider's text.
        assert_eq!(text, "Olá, mundo");
        assert!(request.head.starts_with("post /v1/audio/transcriptions "));
        assert!(request.head.contains("authorization: bearer test-key"));
        assert!(request.head.contains("content-type: multipart/form-data"));
        assert_eq!(request.field("model"), vec!["whisper-1"]);
        assert_eq!(request.field("language"), vec!["pt"]);
        assert_eq!(request.field("prompt"), vec!["Handy, STF"]);

        let body = request.body_text();
        assert!(body.contains("name=\"file\"; filename=\"audio.wav\""));
        assert!(body
            .to_ascii_lowercase()
            .contains("content-type: audio/wav"));
        let riff = request.body.windows(4).position(|w| w == b"RIFF").unwrap();
        let wav = hound::WavReader::new(Cursor::new(&request.body[riff..])).unwrap();
        assert_eq!(wav.spec().sample_rate, 16_000);
        assert_eq!(wav.spec().channels, 1);
        assert_eq!(wav.spec().bits_per_sample, 16);
        assert_eq!(wav.duration() as usize, audio.len());
    }

    #[tokio::test]
    async fn rejected_key_gives_an_actionable_error_without_echoing_the_key() {
        let (base_url, server) = serve_once(
            "401 Unauthorized",
            r#"{"error":{"message":"Incorrect API key provided: sk-proj-****wxyz."}}"#,
        )
        .await;
        let settings = custom_endpoint_settings(&base_url);

        let error = transcribe(&settings, &tone(0.5)).await.unwrap_err();
        server.await.unwrap();

        assert!(error.contains("rejected the API key"), "{error}");
        assert!(error.contains("401"), "{error}");
        assert!(!error.contains("wxyz"), "{error}");
    }

    #[tokio::test]
    async fn provider_errors_keep_the_provider_message() {
        let (base_url, server) = serve_once(
            "404 Not Found",
            r#"{"error":{"message":"The model `whisper-9` does not exist."}}"#,
        )
        .await;
        let settings = custom_endpoint_settings(&base_url);

        let error = transcribe(&settings, &tone(0.5)).await.unwrap_err();
        server.await.unwrap();

        assert!(error.contains("'whisper-1'"), "{error}");
        assert!(
            error.contains("The model `whisper-9` does not exist."),
            "{error}"
        );
    }

    #[tokio::test]
    async fn hosted_providers_require_a_key_before_uploading() {
        let mut settings = get_default_settings();
        settings.cloud_transcription_enabled = true;
        settings.cloud_transcription_provider_id = "openrouter".into();
        settings
            .post_process_api_keys
            .insert("openrouter".into(), "   ".into());

        let error = transcribe(&settings, &tone(0.5)).await.unwrap_err();
        assert!(error.contains("OpenRouter API key"), "{error}");
    }

    #[tokio::test]
    async fn empty_recording_skips_the_request() {
        // No provider is configured at all: an empty recording must not fail.
        let settings = get_default_settings();
        assert_eq!(transcribe(&settings, &[]).await.unwrap(), "");
    }

    #[test]
    fn keys_are_shared_with_post_processing() {
        let mut settings = get_default_settings();
        settings
            .post_process_api_keys
            .insert("openai".into(), " sk-shared \n".into());
        assert_eq!(api_key_for(&settings, "openai"), "sk-shared");
        assert_eq!(api_key_for(&settings, "unknown"), "");
    }

    #[test]
    fn language_intent_maps_to_iso_639_1_or_detection() {
        assert_eq!(language_hint("auto"), None);
        assert_eq!(language_hint("pt"), Some("pt".into()));
        assert_eq!(language_hint("pt-BR"), Some("pt".into()));
        assert_eq!(language_hint("zh-Hant"), Some("zh".into()));
        assert_eq!(language_hint("yue"), None);
        assert_eq!(language_hint(""), None);
    }

    #[test]
    fn gpt_transcribe_uses_languages_and_keywords() {
        let words = vec!["Handy".to_string(), "<b>bold</b>".to_string()];
        let (fields, prompted) = upload_fields("openai", "gpt-transcribe", Some("pt"), &words);
        assert_eq!(
            fields,
            vec![
                ("model", "gpt-transcribe".to_string()),
                ("languages[]", "pt".to_string()),
                ("keywords[]", "Handy".to_string()),
            ]
        );
        assert!(prompted);
        assert!(uses_gpt_transcribe_fields("gpt-transcribe-2026-09-01"));
        assert!(uses_gpt_transcribe_fields("openai/gpt-transcribe"));
        assert!(!uses_gpt_transcribe_fields("gpt-4o-transcribe"));
    }

    #[test]
    fn openrouter_sends_language_but_keeps_vocabulary_local() {
        let words = vec!["Handy".to_string()];
        let (fields, prompted) =
            upload_fields("openrouter", "openai/gpt-transcribe", Some("pt"), &words);
        assert_eq!(
            fields,
            vec![
                ("model", "openai/gpt-transcribe".to_string()),
                ("language", "pt".to_string()),
            ]
        );
        assert!(!prompted);
    }

    #[test]
    fn whisper_style_models_take_language_and_prompt() {
        let (fields, prompted) = upload_fields("groq", "whisper-large-v3-turbo", None, &[]);
        assert_eq!(
            fields,
            vec![("model", "whisper-large-v3-turbo".to_string())]
        );
        assert!(!prompted);
    }

    #[test]
    fn short_audio_is_sent_in_one_piece() {
        let audio = vec![0.1; 100];
        assert_eq!(chunk_ranges(&audio, 100, 30, 10), vec![0..100]);
    }

    #[test]
    fn long_audio_splits_inside_the_quietest_window() {
        // Loud everywhere except a silent stretch shortly before each limit.
        let mut audio = vec![0.5; 250];
        audio[80..90].fill(0.0);
        audio[165..175].fill(0.0);

        let ranges = chunk_ranges(&audio, 100, 30, 10);

        assert_eq!(ranges, vec![0..85, 85..170, 170..250]);
    }

    #[test]
    fn chunks_without_a_quiet_spot_still_cover_everything_within_limits() {
        let audio = vec![0.5; 1_000];
        let ranges = chunk_ranges(&audio, 100, 30, 10);

        assert_eq!(ranges.first().unwrap().start, 0);
        assert_eq!(ranges.last().unwrap().end, audio.len());
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
        assert!(ranges
            .iter()
            .all(|range| !range.is_empty() && range.len() <= 100));
    }

    #[test]
    fn encoded_wav_round_trips_as_16_bit_mono() {
        let audio = vec![0.0, 0.5, -0.5, 1.5, -1.5];
        let wav = encode_wav(&audio).unwrap();
        let mut reader = hound::WavReader::new(Cursor::new(wav)).unwrap();
        let samples: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();
        assert_eq!(samples, vec![0, 16383, -16383, i16::MAX, -i16::MAX]);
    }

    #[test]
    fn model_lists_keep_only_speech_models_for_mixed_catalogues() {
        let listing = serde_json::json!({
            "data": [
                { "id": "gpt-5" },
                { "id": "gpt-transcribe" },
                { "id": "whisper-1" },
                { "id": "gpt-4o-mini-transcribe" },
                { "id": "whisper-1" }
            ]
        });
        assert_eq!(
            transcription_model_ids("openai", &listing),
            vec!["gpt-4o-mini-transcribe", "gpt-transcribe", "whisper-1"]
        );
        // A custom server's listing is taken as-is.
        assert_eq!(
            transcription_model_ids(CLOUD_TRANSCRIPTION_CUSTOM_PROVIDER_ID, &listing),
            vec![
                "gpt-4o-mini-transcribe",
                "gpt-5",
                "gpt-transcribe",
                "whisper-1"
            ]
        );
    }

    #[test]
    fn provider_messages_are_extracted_and_shortened() {
        assert_eq!(
            provider_error_message(r#"{"error":{"message":"bad language"}}"#).as_deref(),
            Some("bad language")
        );
        assert_eq!(
            provider_error_message(r#"{"detail":"Not Found"}"#).as_deref(),
            Some("Not Found")
        );
        assert_eq!(provider_error_message("<html>502</html>"), None);
        assert_eq!(provider_error_message(""), None);
        let long = "x".repeat(MAX_ERROR_DETAIL_CHARS + 50);
        let shortened = provider_error_message(&long).unwrap();
        assert_eq!(shortened.chars().count(), MAX_ERROR_DETAIL_CHARS + 1);
    }
}
