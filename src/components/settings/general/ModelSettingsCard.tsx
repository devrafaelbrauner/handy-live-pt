import React from "react";
import { useTranslation } from "react-i18next";
import { SettingsGroup } from "../../ui/SettingsGroup";
import { LanguageSelector } from "../LanguageSelector";
import { TranslateToEnglish } from "../TranslateToEnglish";
import { useModelStore } from "../../../stores/modelStore";
import { useSettings } from "../../../hooks/useSettings";
import { activeCloudModelName } from "../cloud/cloudProviders";
import type { ModelInfo } from "@/bindings";
import {
  CHINESE_LANGUAGE_CODE,
  getUniqueCapabilityLanguages,
} from "@/lib/constants/languages";

export const ModelSettingsCard: React.FC = () => {
  const { t } = useTranslation();
  const { currentModel, models } = useModelStore();
  const { settings } = useSettings();

  // Cloud models are not limited to a published language list: offer every
  // language plus auto-detect. Translation to English is local-only.
  const cloudModelName = activeCloudModelName(settings, t);
  if (cloudModelName) {
    return (
      <SettingsGroup
        title={t("settings.modelSettings.title", { model: cloudModelName })}
      >
        <LanguageSelector descriptionMode="tooltip" grouped={true} />
      </SettingsGroup>
    );
  }

  const currentModelInfo = models.find((m: ModelInfo) => m.id === currentModel);

  const supportsLanguageSelection =
    currentModelInfo?.supports_language_selection ?? false;
  const capabilityLanguages = getUniqueCapabilityLanguages(
    currentModelInfo?.supported_languages ?? [],
  );
  const supportsChineseOnlyScriptSelection =
    capabilityLanguages.length === 1 &&
    capabilityLanguages[0] === CHINESE_LANGUAGE_CODE;
  const showLanguageSelector =
    supportsLanguageSelection || supportsChineseOnlyScriptSelection;
  const supportsTranslation = currentModelInfo?.supports_translation ?? false;
  const hasAnySettings = showLanguageSelector || supportsTranslation;

  // Don't render anything if no model is selected or no settings available
  if (!currentModel || !currentModelInfo || !hasAnySettings) {
    return null;
  }

  return (
    <SettingsGroup
      title={t("settings.modelSettings.title", {
        model: currentModelInfo.name,
      })}
    >
      {showLanguageSelector && (
        <LanguageSelector
          descriptionMode="tooltip"
          grouped={true}
          supportedLanguages={currentModelInfo.supported_languages}
          supportsLanguageDetection={
            currentModelInfo.supports_language_detection
          }
        />
      )}
      {supportsTranslation && (
        <TranslateToEnglish descriptionMode="tooltip" grouped={true} />
      )}
    </SettingsGroup>
  );
};
