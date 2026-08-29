import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const updaterMocks = vi.hoisted(() => ({
  check: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: updaterMocks.check,
}));

describe("app update checks", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-08-29T14:00:00Z"));
    updaterMocks.check.mockReset();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("shares an in-flight request between overlapping checks", async () => {
    let finish: ((value: null) => void) | undefined;
    updaterMocks.check.mockReturnValue(
      new Promise<null>((resolve) => {
        finish = resolve;
      }),
    );
    const { checkForAppUpdate } = await import("./appUpdater");

    const first = checkForAppUpdate();
    const second = checkForAppUpdate();
    expect(updaterMocks.check).toHaveBeenCalledTimes(1);

    finish?.(null);
    await expect(Promise.all([first, second])).resolves.toEqual([null, null]);
  });

  it("throttles failed checks until the retry interval has elapsed", async () => {
    updaterMocks.check.mockRejectedValueOnce(new Error("offline"));
    const { checkForAppUpdate } = await import("./appUpdater");

    await expect(checkForAppUpdate()).rejects.toThrow("offline");
    await expect(checkForAppUpdate()).resolves.toBeNull();
    expect(updaterMocks.check).toHaveBeenCalledTimes(1);

    vi.advanceTimersByTime(5 * 60_000);
    updaterMocks.check.mockResolvedValueOnce(null);
    await expect(checkForAppUpdate()).resolves.toBeNull();
    expect(updaterMocks.check).toHaveBeenCalledTimes(2);
  });

  it("refreshes cached metadata when a check is forced", async () => {
    const firstUpdate = { version: "0.2.0" } as never;
    const secondUpdate = { version: "0.2.1" } as never;
    updaterMocks.check.mockResolvedValueOnce(firstUpdate).mockResolvedValueOnce(secondUpdate);
    const { checkForAppUpdate } = await import("./appUpdater");

    await expect(checkForAppUpdate()).resolves.toBe(firstUpdate);
    await expect(checkForAppUpdate()).resolves.toBe(firstUpdate);
    expect(updaterMocks.check).toHaveBeenCalledTimes(1);

    await expect(checkForAppUpdate(true)).resolves.toBe(secondUpdate);
    expect(updaterMocks.check).toHaveBeenCalledTimes(2);
  });

  it("invalidates a failed download so the next check gets fresh metadata", async () => {
    const failedUpdate = { version: "0.2.0" } as never;
    const replacementUpdate = { version: "0.2.1" } as never;
    updaterMocks.check.mockResolvedValueOnce(failedUpdate).mockResolvedValueOnce(replacementUpdate);
    const { checkForAppUpdate, invalidateAppUpdate } = await import("./appUpdater");

    await expect(checkForAppUpdate()).resolves.toBe(failedUpdate);
    invalidateAppUpdate(failedUpdate);
    await expect(checkForAppUpdate()).resolves.toBe(replacementUpdate);
    expect(updaterMocks.check).toHaveBeenCalledTimes(2);
  });
});
