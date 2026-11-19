import { describe, expect, it } from "vitest";
import { MOD_PROP } from "@/lib/platform";
import {
  defersToTerminal,
  getBindingTokens,
  originatesInTerminal,
  type KeyBinding,
  matchBinding,
  SHORTCUTS,
} from "./shortcuts";

// These tests run in the vitest node environment, where the Tauri OS plugin is
// unavailable so `IS_MAC` resolves to false. That makes the non-mac token
// branch deterministic across host platforms.

function event(over: Partial<KeyboardEvent>): KeyboardEvent {
  return {
    key: "",
    code: "",
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    metaKey: false,
    ...over,
  } as KeyboardEvent;
}

function surfaceTarget(): EventTarget {
  return {
    closest: (selector: string) =>
      selector === "[data-tarex-terminal-surface]" ? {} : null,
  } as unknown as EventTarget;
}

function terminalTabTarget(): EventTarget {
  return {
    closest: (selector: string) =>
      selector === "[data-terminal-tab]" ? {} : null,
  } as unknown as EventTarget;
}

function outsideTarget(): EventTarget {
  return { closest: () => null } as unknown as EventTarget;
}

describe("getBindingTokens", () => {
  it("returns nothing for an undefined binding", () => {
    expect(getBindingTokens(undefined)).toEqual([]);
  });

  it("lists modifiers in order, then the key", () => {
    const binding: KeyBinding = { key: "k", ctrl: true, shift: true };
    expect(getBindingTokens(binding)).toEqual(["Ctrl", "Shift", "K"]);
  });

  it("labels space and arrow keys", () => {
    expect(getBindingTokens({ key: " ", meta: true })).toEqual([
      "Win",
      "Space",
    ]);
    expect(getBindingTokens({ key: "ArrowUp", alt: true })).toEqual([
      "Alt",
      "↑",
    ]);
  });

  it("uppercases a single-character key", () => {
    expect(getBindingTokens({ key: "c" })).toEqual(["C"]);
  });
});

describe("matchBinding", () => {
  it("matches when key and all modifiers agree", () => {
    expect(
      matchBinding(event({ key: "c", ctrlKey: true }), {
        key: "c",
        ctrl: true,
      }),
    ).toBe(true);
  });

  it("matches the key case-insensitively", () => {
    expect(
      matchBinding(event({ key: "C", ctrlKey: true }), {
        key: "c",
        ctrl: true,
      }),
    ).toBe(true);
  });

  it("fails when a required modifier is missing", () => {
    expect(matchBinding(event({ key: "c" }), { key: "c", ctrl: true })).toBe(
      false,
    );
  });

  it("fails when an extra modifier is pressed", () => {
    expect(
      matchBinding(event({ key: "c", ctrlKey: true, shiftKey: true }), {
        key: "c",
        ctrl: true,
      }),
    ).toBe(false);
  });

  it("falls back to the physical code for alt combinations", () => {
    // Alt often rewrites e.key (here to "ç"); the binding still matches via e.code.
    expect(
      matchBinding(event({ key: "ç", code: "KeyC", altKey: true }), {
        key: "c",
        alt: true,
      }),
    ).toBe(true);
    expect(
      matchBinding(event({ key: "ç", code: "KeyD", altKey: true }), {
        key: "c",
        alt: true,
      }),
    ).toBe(false);
  });

  it("falls back to the physical code for shift combinations", () => {
    // Shift turns Period into ">"; ⌘⇧. still matches the "." binding.
    expect(
      matchBinding(
        event({ key: ">", code: "Period", shiftKey: true, metaKey: true }),
        { key: ".", shift: true, meta: true },
      ),
    ).toBe(true);
    expect(
      matchBinding(
        event({ key: ">", code: "Comma", shiftKey: true, metaKey: true }),
        { key: ".", shift: true, meta: true },
      ),
    ).toBe(false);
  });

  it("does not fall back to the physical code without alt or shift", () => {
    expect(
      matchBinding(event({ key: "ç", code: "KeyC", metaKey: true }), {
        key: "c",
        meta: true,
      }),
    ).toBe(false);
  });

  it("only accepts digit keys for the jump-to-tab shortcut", () => {
    expect(
      matchBinding(event({ key: "3" }), { key: "1" }, "tab.selectByIndex"),
    ).toBe(true);
    expect(
      matchBinding(event({ key: "x" }), { key: "1" }, "tab.selectByIndex"),
    ).toBe(false);
  });
});

type DeferralCase = {
  name: string;
  event: Partial<KeyboardEvent>;
  binding: KeyBinding;
  inside: boolean;
};

const deferralMatrix: DeferralCase[] = [
  {
    name: "Ctrl+R defers",
    event: { key: "r", code: "KeyR", ctrlKey: true },
    binding: { key: "r", ctrl: true },
    inside: false,
  },
  {
    name: "Ctrl+Shift+R still fires",
    event: { key: "R", code: "KeyR", ctrlKey: true, shiftKey: true },
    binding: { key: "r", ctrl: true, shift: true },
    inside: true,
  },
  {
    name: "Cmd+R still fires",
    event: { key: "r", code: "KeyR", metaKey: true },
    binding: { key: "r", meta: true },
    inside: true,
  },
  {
    name: "Ctrl+Tab still fires",
    event: { key: "Tab", code: "Tab", ctrlKey: true },
    binding: { key: "Tab", ctrl: true },
    inside: true,
  },
  {
    name: "Ctrl+Shift+Tab still fires",
    event: { key: "Tab", code: "Tab", ctrlKey: true, shiftKey: true },
    binding: { key: "Tab", ctrl: true, shift: true },
    inside: true,
  },
  {
    name: "Ctrl+C defers",
    event: { key: "c", code: "KeyC", ctrlKey: true },
    binding: { key: "c", ctrl: true },
    inside: false,
  },
  {
    name: "Ctrl+V defers",
    event: { key: "v", code: "KeyV", ctrlKey: true },
    binding: { key: "v", ctrl: true },
    inside: false,
  },
  {
    name: "Ctrl+Z defers",
    event: { key: "z", code: "KeyZ", ctrlKey: true },
    binding: { key: "z", ctrl: true },
    inside: false,
  },
  {
    name: "Ctrl+K defers",
    event: { key: "k", code: "KeyK", ctrlKey: true },
    binding: { key: "k", ctrl: true },
    inside: false,
  },
  {
    name: "Ctrl+B is unchanged",
    event: { key: "b", code: "KeyB", ctrlKey: true },
    binding: { key: "b", ctrl: true },
    inside: true,
  },
  {
    name: "Ctrl+Shift+T still fires",
    event: { key: "T", code: "KeyT", ctrlKey: true, shiftKey: true },
    binding: { key: "t", ctrl: true, shift: true },
    inside: true,
  },
  {
    name: "Ctrl+Alt+R still fires",
    event: { key: "r", code: "KeyR", ctrlKey: true, altKey: true },
    binding: { key: "r", ctrl: true, alt: true },
    inside: true,
  },
  {
    name: "Ctrl+P defers",
    event: { key: "p", code: "KeyP", ctrlKey: true },
    binding: { key: "p", ctrl: true },
    inside: false,
  },
  {
    name: "Ctrl+W defers",
    event: { key: "w", code: "KeyW", ctrlKey: true },
    binding: { key: "w", ctrl: true },
    inside: false,
  },
];

describe("terminal Ctrl deferral", () => {
  it.each(deferralMatrix)(
    "$name: inside=$inside, outside=true",
    ({ event: values, binding, inside }) => {
      expect(
        matchBinding(event({ ...values, target: surfaceTarget() }), binding),
      ).toBe(inside);
      expect(
        matchBinding(
          event({ ...values, target: terminalTabTarget() }),
          binding,
        ),
      ).toBe(inside);
      expect(
        matchBinding(event({ ...values, target: outsideTarget() }), binding),
      ).toBe(true);
    },
  );

  it("defers plain Ctrl letters from the ghostty surface and the terminal tab", () => {
    const values = { key: "r", code: "KeyR", ctrlKey: true };
    const binding: KeyBinding = { key: "r", ctrl: true };
    expect(originatesInTerminal(surfaceTarget())).toBe(true);
    expect(originatesInTerminal(terminalTabTarget())).toBe(true);
    expect(originatesInTerminal(outsideTarget())).toBe(false);
    expect(originatesInTerminal(null)).toBe(false);
    expect(
      defersToTerminal(event({ ...values, target: surfaceTarget() }), binding),
    ).toBe(true);
    expect(
      defersToTerminal(event({ ...values, target: outsideTarget() }), binding),
    ).toBe(false);
  });

  it("never defers chords with extra modifiers or non-letter keys", () => {
    const target = surfaceTarget();
    expect(
      defersToTerminal(
        event({ key: "r", ctrlKey: true, shiftKey: true, target }),
        { key: "r", ctrl: true },
      ),
    ).toBe(false);
    expect(
      defersToTerminal(
        event({ key: "r", ctrlKey: true, metaKey: true, target }),
        {
          key: "r",
          ctrl: true,
        },
      ),
    ).toBe(false);
    expect(
      defersToTerminal(event({ key: "Tab", ctrlKey: true, target }), {
        key: "Tab",
        ctrl: true,
      }),
    ).toBe(false);
    expect(
      defersToTerminal(event({ key: ",", ctrlKey: true, target }), {
        key: ",",
        ctrl: true,
      }),
    ).toBe(false);
    expect(
      defersToTerminal(event({ key: "r", ctrlKey: true, target }), {
        key: "r",
        meta: true,
      }),
    ).toBe(false);
  });
});

describe("SHORTCUTS registry", () => {
  it("binds Mod+S to editor.save", () => {
    const save = SHORTCUTS.find((s) => s.id === "editor.save");
    expect(save).toBeDefined();
    expect(save!.defaultBindings).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ key: "s", [MOD_PROP]: true }),
      ]),
    );
  });
});
