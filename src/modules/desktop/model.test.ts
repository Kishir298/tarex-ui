import {
  bytes,
  joinPath,
  keySequence,
  parentPath,
  processMatches,
  telemetryIntervalMs,
  textOr,
  usagePair,
} from "@/modules/desktop/model";
import { describe, expect, it } from "vitest";

describe("desktop controls", () => {
  it("keeps navigation at filesystem roots and preserves path text", () => {
    expect(parentPath("/")).toBe("/");
    expect(parentPath("C:\\")).toBe("C:/");
    expect(parentPath("C:/Users")).toBe("C:/");
    expect(parentPath("/Users/Ada's work/")).toBe("/Users");
    expect(joinPath("/", "file")).toBe("/file");
    expect(bytes(1024)).toBe("1.0 K");
    expect(bytes(undefined)).toBe("--");
  });
  it("encodes terminal controls, modifiers and shifted punctuation", () => {
    expect(keySequence("ENTER", false, false, false)).toBe("\r");
    expect(keySequence("c", false, true, false)).toBe("\x03");
    expect(keySequence("1", true, false, false)).toBe("!");
    expect(keySequence("x", false, false, true)).toBe("\x1bx");
    expect(keySequence("a", true, false, false, true)).toBe("a");
    expect(keySequence("LEFT", false, false, false)).toBe("\x1b[D");
    expect(keySequence("SHIFT", false, false, false)).toBe("");
  });
});

describe("honest hardware states", () => {
  it("never renders unreported text as a measurement", () => {
    expect(textOr("Apple M3")).toBe("Apple M3");
    expect(textOr("   ")).toBe("--");
    expect(textOr("")).toBe("--");
    expect(textOr(null)).toBe("--");
    expect(textOr(undefined)).toBe("--");
    expect(textOr(null, "Unavailable")).toBe("Unavailable");
  });
  it("shows a dash instead of zero totals that were never reported", () => {
    expect(usagePair(4 * 1024 ** 3, 16 * 1024 ** 3)).toBe("4.0 G / 16.0 G");
    expect(usagePair(0, 0)).toBe("--");
    expect(usagePair(10, 0)).toBe("--");
    expect(usagePair(undefined, 1024)).toBe("-- / 1.0 K");
    expect(usagePair(1, undefined)).toBe("--");
  });
});

describe("efficiency mode", () => {
  it("slows Tarex's own sampling interval only when enabled", () => {
    expect(telemetryIntervalMs(false)).toBe(5_000);
    expect(telemetryIntervalMs(true)).toBe(15_000);
    expect(telemetryIntervalMs(true)).toBeGreaterThan(
      telemetryIntervalMs(false),
    );
  });
});

describe("process filter", () => {
  const process = { pid: 4242, name: "node" };
  it("matches by name or pid and treats a blank query as no filter", () => {
    expect(processMatches(process, "")).toBe(true);
    expect(processMatches(process, "   ")).toBe(true);
    expect(processMatches(process, "NOD")).toBe(true);
    expect(processMatches(process, "4242")).toBe(true);
    expect(processMatches(process, "42")).toBe(true);
    expect(processMatches(process, "python")).toBe(false);
    expect(processMatches(process, "9999")).toBe(false);
  });
});
