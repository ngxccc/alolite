import type { AppSettings } from "@/shared/types/ipc.generated";

export interface SettingsTabProps {
  settings: AppSettings;
  updateSettings: (patch: Partial<AppSettings>) => void;
}
