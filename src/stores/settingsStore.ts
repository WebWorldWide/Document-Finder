import { create } from "zustand";
import { persist } from "zustand/middleware";
import { ALL_SOURCES, type SourceId } from "@/lib/utils";

interface SettingsState {
  selectedSources: SourceId[];
  perSource: number;
  maxTotal: number;
  concurrency: number;
  libraryRoot: string;

  toggleSource: (s: SourceId) => void;
  set: (patch: Partial<Omit<SettingsState, "toggleSource" | "set">>) => void;
}

export const useSettings = create<SettingsState>()(
  persist(
    (set) => ({
      selectedSources: [...ALL_SOURCES] as SourceId[],
      perSource: 25,
      maxTotal: 150,
      concurrency: 6,
      libraryRoot: "",
      toggleSource: (s) =>
        set((state) => ({
          selectedSources: state.selectedSources.includes(s)
            ? state.selectedSources.filter((x) => x !== s)
            : [...state.selectedSources, s],
        })),
      set: (patch) => set(patch),
    }),
    { name: "document-finder-settings" },
  ),
);
