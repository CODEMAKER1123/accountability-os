import { describe, expect, it } from "vitest";

import { addDaysISO, fmtClockDuration, fmtDuration, fmtPct, fmtScore } from "./time";

describe("fmtDuration", () => {
  it("formats hours and minutes", () => {
    expect(fmtDuration(4 * 3600 + 15 * 60)).toBe("4h 15m");
    expect(fmtDuration(58 * 60)).toBe("58m");
    expect(fmtDuration(42)).toBe("42s");
    expect(fmtDuration(0)).toBe("0s");
    expect(fmtDuration(-5)).toBe("0s");
  });
});

describe("fmtClockDuration", () => {
  it("formats a running timer", () => {
    expect(fmtClockDuration(4472)).toBe("01:14:32");
    expect(fmtClockDuration(59)).toBe("00:59");
    expect(fmtClockDuration(600)).toBe("10:00");
  });
});

describe("scores", () => {
  it("renders missing values as em dash", () => {
    expect(fmtScore(null)).toBe("—");
    expect(fmtPct(undefined)).toBe("—");
    expect(fmtScore(77.6)).toBe("78");
    expect(fmtPct(66.4)).toBe("66%");
  });
});

describe("addDaysISO", () => {
  it("crosses month boundaries", () => {
    expect(addDaysISO("2026-08-29", 3)).toBe("2026-09-01");
    expect(addDaysISO("2026-01-01", -1)).toBe("2025-12-31");
  });
});
