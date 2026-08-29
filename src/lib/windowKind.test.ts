import { describe, expect, it } from "vitest";

import { resolveWindowKind } from "./windowKind";

describe("auxiliary window routing", () => {
  it("routes production windows by their Tauri labels", () => {
    expect(resolveWindowKind("", "widget")).toBe("widget");
    expect(resolveWindowKind("", "intervention")).toBe("popup");
    expect(resolveWindowKind("", "capture")).toBe("capture");
  });

  it("never lets a query string replace the main Tauri window", () => {
    expect(resolveWindowKind("?window=widget", "main")).toBeNull();
  });

  it("keeps query routing for browser previews", () => {
    expect(resolveWindowKind("?window=widget")).toBe("widget");
    expect(resolveWindowKind("?window=popup")).toBe("popup");
    expect(resolveWindowKind("?window=capture")).toBe("capture");
    expect(resolveWindowKind("?window=unknown")).toBeNull();
  });
});
