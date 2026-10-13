import type { TFunction } from "i18next";
import type { AppSettings, CloudTranscriptionProvider } from "@/bindings";

/** Mirrors `CLOUD_TRANSCRIPTION_CUSTOM_PROVIDER_ID` in src-tauri/src/settings.rs. */
export const CUSTOM_CLOUD_PROVIDER_ID = "custom_transcription";

/** Where each hosted provider issues API keys. */
export const API_KEY_PAGES: Partial<Record<string, string>> = {
  openai: "https://platform.openai.com/api-keys",
  openrouter: "https://openrouter.ai/settings/keys",
  groq: "https://console.groq.com/keys",
};

/** Brand names come from the backend; only the custom endpoint is translated. */
export const cloudProviderLabel = (
  provider: CloudTranscriptionProvider,
  t: TFunction,
): string =>
  provider.id === CUSTOM_CLOUD_PROVIDER_ID
    ? t("settings.cloud.transcription.provider.custom")
    : provider.label;

/**
 * "Provider · model" for the active cloud transcription setup, or `null` while
 * recordings are transcribed locally.
 */
export const activeCloudModelName = (
  settings: AppSettings | null,
  t: TFunction,
): string | null => {
  if (!settings?.cloud_transcription_enabled) return null;
  const providerId = settings.cloud_transcription_provider_id ?? "";
  const provider = settings.cloud_transcription_providers?.find(
    (candidate) => candidate.id === providerId,
  );
  return t("modelSelector.cloud", {
    provider: provider ? cloudProviderLabel(provider, t) : providerId,
    model: settings.cloud_transcription_models?.[providerId] ?? "",
  });
};
