import { useState } from "react";

export function useLogPreference(
  name: "analysis" | "charts",
  fallback: boolean,
) {
  const key = `lumagate.logs.${name}`;
  const [value, setValue] = useState(() => {
    try {
      let saved = window.localStorage.getItem(key);
      const legacy = `cc-switch-manual.logs.${name}`;
      if (saved === null) {
        saved = window.localStorage.getItem(legacy);
        if (saved !== null) {
          window.localStorage.setItem(key, saved);
          window.localStorage.removeItem(legacy);
        }
      }
      return saved === "true" ? true : saved === "false" ? false : fallback;
    } catch {
      return fallback;
    }
  });
  const update = (next: boolean) => {
    setValue(next);
    try {
      window.localStorage.setItem(key, String(next));
    } catch {
      /* usable without storage */
    }
  };
  return [value, update] as const;
}
