import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/** The text field that has keyboard focus, if any. */
export function focusedField(): HTMLTextAreaElement | HTMLInputElement | null {
  const el = document.activeElement;
  if (el instanceof HTMLTextAreaElement) return el;
  if (el instanceof HTMLInputElement && /^(text|search)$/.test(el.type)) return el;
  return null;
}

/** Inserts text at the cursor of a field (keeps undo where the browser supports it). */
export function insertAtCursor(el: HTMLTextAreaElement | HTMLInputElement, text: string) {
  el.focus();
  const start = el.selectionStart ?? el.value.length;
  const before = el.value.slice(0, start);
  const sep = before && !/\s$/.test(before) ? " " : "";
  if (!document.execCommand("insertText", false, sep + text)) {
    el.setRangeText(sep + text, start, el.selectionEnd ?? start, "end");
    el.dispatchEvent(new Event("input", { bubbles: true }));
  }
}
