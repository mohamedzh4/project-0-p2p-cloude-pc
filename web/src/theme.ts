import { useEffect, useState } from "react";

/** Night is the default look; Day is for bright rooms. "system" follows the OS. */
export type ThemeChoice = "system" | "night" | "day";

const KEY = "dcl-theme";

function readChoice(): ThemeChoice {
  try {
    const stored = window.localStorage.getItem(KEY);
    if (stored === "night" || stored === "day" || stored === "system") {
      return stored;
    }
  } catch {
    // Storage can be blocked; the default look still works.
  }
  return "night";
}

export function useTheme(): [ThemeChoice, (choice: ThemeChoice) => void] {
  const [choice, setChoice] = useState<ThemeChoice>(readChoice);

  useEffect(() => {
    const root = document.documentElement;
    if (choice === "system") {
      root.removeAttribute("data-theme");
    } else {
      root.setAttribute("data-theme", choice);
    }
    try {
      window.localStorage.setItem(KEY, choice);
    } catch {
      // Not saved; the choice still applies for this visit.
    }
  }, [choice]);

  return [choice, setChoice];
}
