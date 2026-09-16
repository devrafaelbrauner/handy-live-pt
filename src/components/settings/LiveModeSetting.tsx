import React, { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { LiveMode } from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { Dropdown, type DropdownOption } from "@/components/ui/Dropdown";
import { SettingContainer } from "@/components/ui/SettingContainer";

interface LiveModeSettingProps {
  descriptionMode?: "tooltip" | "inline";
  grouped?: boolean;
}

/**
 * Live text mode for batch (non-streaming) models. "standard" is the default
 * and keeps today's behaviour. "preview" shows disposable live captions that
 * grow at each speech pause; the pasted text still comes from transcribing the
 * whole recording. "chunks_paste" is reserved and not selectable yet.
 */
export const LiveModeSetting: React.FC<LiveModeSettingProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateSetting, isUpdating } = useSettings();
  const selectedMode = getSetting("live_mode") ?? "standard";

  const options = useMemo<DropdownOption[]>(
    () => [
      {
        value: "standard",
        label: t("settings.advanced.liveMode.options.standard"),
      },
      {
        value: "preview",
        label: t("settings.advanced.liveMode.options.preview"),
      },
      {
        value: "chunks_paste",
        label: t("settings.advanced.liveMode.options.chunksPaste"),
        disabled: true,
      },
    ],
    [t],
  );

  return (
    <SettingContainer
      title={t("settings.advanced.liveMode.label")}
      description={t("settings.advanced.liveMode.description")}
      descriptionMode={descriptionMode}
      grouped={grouped}
      layout="horizontal"
    >
      <Dropdown
        options={options}
        selectedValue={selectedMode}
        onSelect={(value) => updateSetting("live_mode", value as LiveMode)}
        disabled={isUpdating("live_mode")}
      />
    </SettingContainer>
  );
};
