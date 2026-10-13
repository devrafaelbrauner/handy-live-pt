//! Settings commands for cloud transcription. API keys go to the key store
//! shared with post-processing (`AppSettings::post_process_api_keys`).

use crate::settings::{self, AppSettings, CloudTranscriptionProvider};
use tauri::AppHandle;

fn require_provider<'a>(
    settings: &'a AppSettings,
    provider_id: &str,
) -> Result<&'a CloudTranscriptionProvider, String> {
    settings
        .cloud_transcription_provider(provider_id)
        .ok_or_else(|| format!("Cloud transcription provider '{provider_id}' not found"))
}

#[tauri::command]
#[specta::specta]
pub fn change_cloud_transcription_enabled_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.cloud_transcription_enabled = enabled;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_cloud_transcription_provider(app: AppHandle, provider_id: String) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    require_provider(&settings, &provider_id)?;
    settings.cloud_transcription_provider_id = provider_id;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_cloud_transcription_api_key_setting(
    app: AppHandle,
    provider_id: String,
    api_key: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    require_provider(&settings, &provider_id)?;
    settings
        .post_process_api_keys
        .insert(provider_id, api_key.trim().to_string());
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_cloud_transcription_model_setting(
    app: AppHandle,
    provider_id: String,
    model: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    require_provider(&settings, &provider_id)?;
    settings
        .cloud_transcription_models
        .insert(provider_id, model.trim().to_string());
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_cloud_transcription_base_url_setting(
    app: AppHandle,
    provider_id: String,
    base_url: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    let provider = settings
        .cloud_transcription_provider_mut(&provider_id)
        .ok_or_else(|| format!("Cloud transcription provider '{provider_id}' not found"))?;
    if !provider.allow_base_url_edit {
        return Err(format!(
            "The base URL of {} cannot be changed",
            provider.label
        ));
    }

    let base_url = base_url.trim().trim_end_matches('/');
    let is_http_url = reqwest::Url::parse(base_url)
        .map(|url| matches!(url.scheme(), "http" | "https"))
        .unwrap_or(false);
    if !is_http_url {
        return Err("Enter a base URL that starts with http:// or https://".to_string());
    }

    provider.base_url = base_url.to_string();
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn fetch_cloud_transcription_models(
    app: AppHandle,
    provider_id: String,
) -> Result<Vec<String>, String> {
    let settings = settings::get_settings(&app);
    let provider = require_provider(&settings, &provider_id)?;
    let api_key = crate::cloud_transcription::api_key_for(&settings, &provider_id);
    // OpenRouter publishes its catalogue and a custom server may run without
    // authentication; OpenAI and Groq only list models for a valid key.
    if api_key.is_empty() && matches!(provider.id.as_str(), "openai" | "groq") {
        return Err(format!(
            "Add your {} API key to list its models.",
            provider.label
        ));
    }
    crate::cloud_transcription::fetch_models(provider, api_key).await
}
