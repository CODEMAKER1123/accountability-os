import { describe, expect, it } from "vitest";

import { parseTheme } from "./theme";

describe("theme preferences", () => {
  it("accepts light and defaults every other value to dark", () => {
    expect(parseTheme("light")).toBe("light");
    expect(parseTheme("dark")).toBe("dark");
    expect(parseTheme("system")).toBe("dark");
    expect(parseTheme(null)).toBe("dark");
    expect(parseTheme(undefined)).toBe("dark");
  });
});
