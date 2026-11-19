import { useEffect, useRef } from "react";
import { usePreferencesStore } from "@/modules/settings/preferences";
import {
  SHORTCUTS,
  matchBinding,
  type KeyBinding,
  type Shortcut,
  type ShortcutId,
} from "../shortcuts";

export type ShortcutHandler = (e: KeyboardEvent) => void;
export type ShortcutHandlers = Partial<Record<ShortcutId, ShortcutHandler>>;

export type ShortcutBindings = Partial<Record<ShortcutId, KeyBinding[]>>;

export type UseGlobalShortcutsOptions = {
  isDisabled?: (id: ShortcutId, e: KeyboardEvent) => boolean;
};

export function findTriggeredShortcut(
  e: KeyboardEvent,
  userShortcuts: ShortcutBindings,
): Shortcut | null {
  for (const s of SHORTCUTS) {
    if (e.repeat && !s.allowRepeat) continue;
    const bindings = userShortcuts[s.id] || s.defaultBindings;
    if (bindings.some((b) => matchBinding(e, b, s.id))) return s;
  }
  return null;
}

export function useGlobalShortcuts(
  handlers: ShortcutHandlers,
  options?: UseGlobalShortcutsOptions,
) {
  const latest = useRef({ handlers, options });
  latest.current = { handlers, options };

  // Access the shortcuts from the store
  const userShortcuts = usePreferencesStore((s) => s.shortcuts);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const { handlers, options } = latest.current;
      const shortcut = findTriggeredShortcut(e, userShortcuts);
      if (!shortcut) return;
      if (options?.isDisabled?.(shortcut.id, e)) return;
      const h = handlers[shortcut.id];
      if (!h) return;
      e.preventDefault();
      e.stopImmediatePropagation();
      h(e);
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () =>
      window.removeEventListener("keydown", onKey, { capture: true });
  }, [userShortcuts]);
}
