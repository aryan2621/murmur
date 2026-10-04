// Which half of the app (dictation or the assistant) Insights and History show, remembered per page.
import { useState } from "react";

export type View = "dictation" | "assistant";
type Page = "insights" | "history";

const storageKey = (page: Page) => `murmur.view.${page}`;

export function getView(page: Page): View {
  try {
    return localStorage.getItem(storageKey(page)) === "assistant" ? "assistant" : "dictation";
  } catch {
    return "dictation";
  }
}

export function setView(page: Page, view: View) {
  try {
    localStorage.setItem(storageKey(page), view);
  } catch {
    /* storage unavailable: the page just opens on dictation */
  }
}

export function useView(page: Page) {
  const [view, set] = useState<View>(() => getView(page));
  return [view, (v: View) => (setView(page, v), set(v))] as const;
}
