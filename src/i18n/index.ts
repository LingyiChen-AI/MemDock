import zh from "./zh.json";
import en from "./en.json";

export type Language = "zh" | "en";
export type Messages = typeof zh;

const dictionaries: Record<Language, Messages> = { zh, en };

export function getMessages(language: Language): Messages {
  return dictionaries[language];
}

/** Empty/"system" follows the OS language: anything starting with "zh" is Chinese, else English. */
export function resolveLanguage(setting: string | null | undefined, navigatorLanguage: string): Language {
  const source = !setting || setting === "system" ? navigatorLanguage : setting;
  return source.toLowerCase().startsWith("zh") ? "zh" : "en";
}

export function htmlLang(language: Language): string {
  return language === "zh" ? "zh-CN" : "en";
}
