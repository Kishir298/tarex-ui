import { describe, expect, it } from "vitest";
import {
  findTriggeredShortcut,
  type ShortcutBindings,
} from "./useGlobalShortcuts";

function event(over: Partial<KeyboardEvent>): KeyboardEvent {
  return {
    key: "",
    code: "",
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    metaKey: false,
    repeat: false,
    ...over,
  } as KeyboardEvent;
}

function terminalTarget(): EventTarget {
  return {
    closest: (selector: string) =>
      selector === "[data-tarex-terminal-surface]" ||
      selector === "[data-terminal-tab]"
        ? {}
        : null,
  } as unknown as EventTarget;
}

function outsideTarget(): EventTarget {
  return { closest: () => null } as unknown as EventTarget;
}

const inside = (over: Partial<KeyboardEvent>): KeyboardEvent =>
  event({ ...over, target: terminalTarget() });
const outside = (over: Partial<KeyboardEvent>): KeyboardEvent =>
  event({ ...over, target: outsideTarget() });

describe("findTriggeredShortcut terminal deferral", () => {
  it.each([
    ["commandPalette.open", { key: "p", code: "KeyP", ctrlKey: true }],
    ["tab.newPrivate", { key: "r", code: "KeyR", ctrlKey: true }],
    ["tab.newEditor", { key: "e", code: "KeyE", ctrlKey: true }],
    ["pane.splitRight", { key: "d", code: "KeyD", ctrlKey: true }],
    ["search.focus", { key: "f", code: "KeyF", ctrlKey: true }],
    ["editor.undo", { key: "z", code: "KeyZ", ctrlKey: true }],
    ["editor.redo", { key: "y", code: "KeyY", ctrlKey: true }],
    ["tab.close", { key: "w", code: "KeyW", ctrlKey: true }],
  ] as const)(
    "defers the plain Ctrl binding of %s inside a terminal",
    (id, values) => {
      expect(findTriggeredShortcut(inside(values), {})).toBeNull();
      expect(findTriggeredShortcut(outside(values), {})).toMatchObject({ id });
    },
  );

  it("keeps Ctrl+Shift chords firing inside a terminal", () => {
    expect(
      findTriggeredShortcut(
        inside({ key: "T", code: "KeyT", ctrlKey: true, shiftKey: true }),
        {},
      ),
    ).toMatchObject({ id: "tab.newBlock" });
    expect(
      findTriggeredShortcut(
        inside({ key: "P", code: "KeyP", ctrlKey: true, shiftKey: true }),
        {},
      ),
    ).toMatchObject({ id: "commandPalette.content" });
  });

  it("keeps user Ctrl+Shift bindings firing inside a terminal", () => {
    const user: ShortcutBindings = {
      "tab.newPrivate": [{ key: "r", ctrl: true, shift: true }],
    };
    expect(
      findTriggeredShortcut(
        inside({ key: "R", code: "KeyR", ctrlKey: true, shiftKey: true }),
        user,
      ),
    ).toMatchObject({ id: "tab.newPrivate" });
  });

  it("keeps Cmd bindings firing inside a terminal", () => {
    const user: ShortcutBindings = {
      "tab.newPrivate": [{ key: "r", meta: true }],
    };
    expect(
      findTriggeredShortcut(
        inside({ key: "r", code: "KeyR", metaKey: true }),
        user,
      ),
    ).toMatchObject({ id: "tab.newPrivate" });
    expect(
      findTriggeredShortcut(
        inside({ key: "r", code: "KeyR", ctrlKey: true }),
        user,
      ),
    ).toBeNull();
  });

  it("keeps Ctrl+Tab and Ctrl+Shift+Tab firing inside a terminal", () => {
    expect(
      findTriggeredShortcut(
        inside({ key: "Tab", code: "Tab", ctrlKey: true }),
        {},
      ),
    ).toMatchObject({ id: "tab.next" });
    expect(
      findTriggeredShortcut(
        inside({ key: "Tab", code: "Tab", ctrlKey: true, shiftKey: true }),
        {},
      ),
    ).toMatchObject({ id: "tab.prev" });
  });

  it("leaves Ctrl+B to the App.tsx disabled-hook path, unchanged", () => {
    expect(
      findTriggeredShortcut(
        inside({ key: "b", code: "KeyB", ctrlKey: true }),
        {},
      ),
    ).toMatchObject({ id: "sidebar.toggle" });
    expect(
      findTriggeredShortcut(
        inside({ key: "B", code: "KeyB", ctrlKey: true, shiftKey: true }),
        {},
      ),
    ).toMatchObject({ id: "sidebar.toggle" });
    expect(
      findTriggeredShortcut(
        outside({ key: "b", code: "KeyB", ctrlKey: true }),
        {},
      ),
    ).toMatchObject({ id: "sidebar.toggle" });
  });

  it("still fires the whole keymap outside a terminal", () => {
    expect(
      findTriggeredShortcut(
        outside({ key: "t", code: "KeyT", ctrlKey: true }),
        {},
      ),
    ).toMatchObject({ id: "tab.new" });
    expect(
      findTriggeredShortcut(
        outside({ key: ",", code: "Comma", ctrlKey: true }),
        {},
      ),
    ).toMatchObject({ id: "settings.open" });
    expect(
      findTriggeredShortcut(
        outside({ key: "p", code: "KeyP", ctrlKey: true, shiftKey: true }),
        {},
      ),
    ).toMatchObject({ id: "commandPalette.content" });
  });
});
