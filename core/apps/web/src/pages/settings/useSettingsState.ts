import { useState } from "react";
import type { SectionId } from "../SettingsPage.types";

type UseSettingsStateArgs = {
  initialActive: SectionId;
};

export function useSettingsState({ initialActive }: UseSettingsStateArgs) {
  const [active, setActive] = useState<SectionId>(initialActive);
  const [query, setQuery] = useState("");

  return {
    active,
    setActive,
    query,
    setQuery,
  };
}
