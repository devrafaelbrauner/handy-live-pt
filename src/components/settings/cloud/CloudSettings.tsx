import React from "react";
import { useTranslation } from "react-i18next";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ExternalLink, RefreshCcw } from "lucide-react";
import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";
import { ResetButton } from "../../ui/ResetButton";
import { SettingContainer, SettingsGroup, ToggleSwitch } from "../../ui";
import { useSettings } from "../../../hooks/useSettings";
import { ApiKeyField } from "../PostProcessingSettingsApi/ApiKeyField";
import { BaseUrlField } from "../PostProcessingSettingsApi/BaseUrlField";
import { ModelSelect } from "../PostProcessingSettingsApi/ModelSelect";
import { ProviderSelect } from "../PostProcessingSettingsApi/ProviderSelect";
import { useCloudTranscriptionState } from "./useCloudTranscriptionState";

export const CloudSettings: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const state = useCloudTranscriptionState();
  const enabled = getSetting("cloud_transcription_enabled") ?? false;
  const apiKeyPage = state.apiKeyPage;

  return (
    <div className="max-w-3xl w-full mx-auto space-y-6">
      <SettingsGroup title={t("settings.cloud.transcription.title")}>
        <ToggleSwitch
          checked={enabled}
          onChange={(checked) =>
            updateSetting("cloud_transcription_enabled", checked)
          }
          isUpdating={isUpdating("cloud_transcription_enabled")}
          label={t("settings.cloud.transcription.enabled.label")}
          description={t("settings.cloud.transcription.enabled.description")}
          descriptionMode="inline"
          grouped={true}
        />

        <SettingContainer
          title={t("settings.cloud.transcription.provider.title")}
          description={t("settings.cloud.transcription.provider.description")}
          descriptionMode="tooltip"
          layout="horizontal"
          grouped={true}
        >
          <div className="flex items-center gap-2">
            <ProviderSelect
              options={state.providerOptions}
              value={state.selectedProviderId}
              onChange={state.selectProvider}
              disabled={state.isPending("provider")}
            />
          </div>
        </SettingContainer>

        {state.isCustomProvider && (
          <SettingContainer
            title={t("settings.cloud.transcription.baseUrl.title")}
            description={t("settings.cloud.transcription.baseUrl.description")}
            descriptionMode="tooltip"
            layout="stacked"
            grouped={true}
          >
            <div className="flex items-center gap-2">
              <BaseUrlField
                key={state.baseUrlRevision}
                value={state.baseUrl}
                onBlur={state.updateBaseUrl}
                placeholder={t(
                  "settings.cloud.transcription.baseUrl.placeholder",
                )}
                disabled={state.isPending("baseUrl")}
              />
            </div>
          </SettingContainer>
        )}

        <SettingContainer
          title={t("settings.cloud.transcription.apiKey.title")}
          description={t("settings.cloud.transcription.apiKey.description")}
          descriptionMode="tooltip"
          layout="stacked"
          grouped={true}
        >
          {/* The key link wraps under the field when the window is narrow. */}
          <div className="flex flex-wrap items-center gap-2">
            <ApiKeyField
              value={state.apiKey}
              onBlur={state.updateApiKey}
              placeholder={t("settings.cloud.transcription.apiKey.placeholder")}
              disabled={state.isPending("apiKey")}
            />
            {apiKeyPage && (
              <Button
                variant="secondary"
                size="sm"
                onClick={() => void openUrl(apiKeyPage)}
                className="flex items-center gap-1 whitespace-nowrap"
              >
                {t("settings.cloud.transcription.apiKey.getKey")}
                <ExternalLink className="h-3 w-3" />
              </Button>
            )}
          </div>
        </SettingContainer>

        <SettingContainer
          title={t("settings.cloud.transcription.model.title")}
          description={t("settings.cloud.transcription.model.description")}
          descriptionMode="tooltip"
          layout="stacked"
          grouped={true}
        >
          <div className="flex items-center gap-2">
            <ModelSelect
              value={state.model}
              options={state.modelOptions}
              disabled={state.isPending("model")}
              isLoading={state.isPending("models")}
              placeholder={
                state.modelOptions.length > 0
                  ? t(
                      "settings.cloud.transcription.model.placeholderWithOptions",
                    )
                  : t("settings.cloud.transcription.model.placeholderNoOptions")
              }
              onSelect={state.updateModel}
              onCreate={state.updateModel}
              onBlur={() => {}}
              className="flex-1 min-w-[380px]"
            />
            <ResetButton
              onClick={state.refreshModels}
              disabled={state.isPending("models")}
              ariaLabel={t("settings.cloud.transcription.model.refreshModels")}
              className="flex h-10 w-10 items-center justify-center"
            >
              <RefreshCcw
                className={`h-4 w-4 ${state.isPending("models") ? "animate-spin" : ""}`}
              />
            </ResetButton>
          </div>
        </SettingContainer>

        {enabled && state.missingApiKey && (
          <Alert variant="warning" contained>
            {t("settings.cloud.transcription.missingApiKey", {
              provider: state.providerLabel,
            })}
          </Alert>
        )}
      </SettingsGroup>
    </div>
  );
};
