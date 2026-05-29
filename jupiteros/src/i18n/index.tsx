import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import { en } from "./en";
import { it } from "./it";

export type Lang = "it" | "en";

const DICTS: Record<Lang, Record<string, string>> = { en, it };
const STORAGE_KEY = "jupiteros-lang";

function detectInitialLang(): Lang {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === "it" || saved === "en") return saved;
  } catch {
    /* localStorage may be unavailable */
  }
  const nav = (typeof navigator !== "undefined" && navigator.language) || "en";
  return nav.toLowerCase().startsWith("it") ? "it" : "en";
}

type Vars = Record<string, string | number>;

/** Translate `key` for the given language, with `{{var}}` interpolation and a
 * simple plural form: when `vars.count` is passed and not 1, prefer `key_plural`. */
export function translate(lang: Lang, key: string, vars?: Vars): string {
  const dict = DICTS[lang];
  let lookup = key;
  if (vars && typeof vars.count === "number" && vars.count !== 1 && (`${key}_plural` in dict)) {
    lookup = `${key}_plural`;
  }
  // Active language, then English fallback, then the raw key (so missing keys are visible).
  let str = dict[lookup] ?? DICTS.en[lookup] ?? dict[key] ?? DICTS.en[key] ?? key;
  if (vars) {
    str = str.replace(/\{\{(\w+)\}\}/g, (_, name) =>
      name in vars ? String(vars[name]) : `{{${name}}}`
    );
  }
  return str;
}

interface LanguageContextValue {
  lang: Lang;
  setLang: (l: Lang) => void;
  t: (key: string, vars?: Vars) => string;
}

const LanguageContext = createContext<LanguageContextValue | null>(null);

export function LanguageProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(detectInitialLang);

  useEffect(() => {
    try {
      document.documentElement.lang = lang;
    } catch {
      /* noop */
    }
  }, [lang]);

  const setLang = useCallback((l: Lang) => {
    setLangState(l);
    try {
      localStorage.setItem(STORAGE_KEY, l);
    } catch {
      /* noop */
    }
  }, []);

  const t = useCallback((key: string, vars?: Vars) => translate(lang, key, vars), [lang]);

  return (
    <LanguageContext.Provider value={{ lang, setLang, t }}>
      {children}
    </LanguageContext.Provider>
  );
}

export function useT(): LanguageContextValue {
  const ctx = useContext(LanguageContext);
  if (!ctx) throw new Error("useT must be used within a LanguageProvider");
  return ctx;
}
