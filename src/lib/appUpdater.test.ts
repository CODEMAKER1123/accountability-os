import { describe, expect, it } from "vitest";

import { EMPTY_UPDATE_PROGRESS, reduceDownloadProgress } from "./appUpdater";

describe("updater download progress", () => {
  it("tracks a known-size download and caps the percentage", () => {
    const started = reduceDownloadProgress(EMPTY_UPDATE_PROGRESS, {
      event: "Started",
      data: { contentLength: 100 },
    });
    const halfway = reduceDownloadProgress(started, {
      event: "Progress",
      data: { chunkLength: 50 },
    });
    const capped = reduceDownloadProgress(halfway, {
      event: "Progress",
      data: { chunkLength: 75 },
    });

    expect(halfway).toMatchObject({ downloaded: 50, total: 100, percent: 50 });
    expect(capped).toMatchObject({ downloaded: 125, total: 100, percent: 100 });
  });

  it("supports unknown download sizes and marks completion", () => {
    const started = reduceDownloadProgress(EMPTY_UPDATE_PROGRESS, {
      event: "Started",
      data: {},
    });
    const progressed = reduceDownloadProgress(started, {
      event: "Progress",
      data: { chunkLength: 25 },
    });
    const finished = reduceDownloadProgress(progressed, { event: "Finished" });

    expect(progressed).toMatchObject({ downloaded: 25, total: null, percent: null });
    expect(finished).toMatchObject({ percent: 100, finished: true });
  });
});
