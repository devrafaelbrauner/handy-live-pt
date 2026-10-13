import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "@/hooks/useSettings";
import { useModelStore } from "@/stores/modelStore";
import { getTranslatedModelName } from "@/lib/utils/modelTranslation";
import { Dropdown, type DropdownOption } from "@/components/ui/Dropdown";
import { SettingContainer } from "@/components/ui/SettingContainer";

interface LiveModelSettingProps {
  descriptionMode?: "tooltip" | "inline";
  grouped?: boolean;
}

/** Value of the disabled entry shown when no downloaded model can stream. */
const NO_STREAMING_MODEL = "__none__";

/**
 * Live model: a natively streaming model that shows disposable live text while
 * the transcription model (local or cloud) has none of its own. "" is off.
 * Only downloaded transcribe-cpp models that support live streaming are
 * offered; the pasted text always comes from the transcription model.
 */
export const LiveModelSetting: React.FC<LiveModelSettingProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const models = useModelStore((state) => state.models);
  const liveModel = getSetting("live_model") ?? "";

  const options = useMemo<DropdownOption[]>(() => {
    const off: DropdownOption = {
      value: "",
      label: t("settings.advanced.liveModel.off"),
    };
    const streamingModels = models.filter(
      (model) =>
        model.is_downloaded &&
        model.supports_streaming &&
        model.engine_type === "TranscribeCpp",
    );
    if (streamingModels.length === 0) {
      return [
        off,
        {
          value: NO_STREAMING_MODEL,
          label: t("settings.advanced.liveModel.none"),
          disabled: true,
        },
      ];
    }
    return [
      off,
      ...streamingModels.map((model) => ({
        value: model.id,
        label: getTranslatedModelName(model, t),
      })),
    ];
  }, [models, t]);

  // A live model that is no longer offered (e.g. deleted) gives no live text,
  // so it reads as off.
  const selectedValue = options.some((option) => option.value === liveModel)
    ? liveModel
    : "";

  return (
    <SettingContainer
      title={t("settings.advanced.liveModel.label")}
      description={t("settings.advanced.liveModel.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
      layout="horizontal"
    >
      <Dropdown
        options={options}
        selectedValue={selectedValue}
        onSelect={(value) => updateSetting("live_model", value)}
        disabled={isUpdating("live_model")}
      />
    </SettingContainer>
  );
};
