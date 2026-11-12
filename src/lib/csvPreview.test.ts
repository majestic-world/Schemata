import { describe, expect, it } from "vitest";
import type { CsvReadOptions } from "../api/types";
import { csvSource, loadingPreview, previewFor, settlePreview } from "./csvPreview";

const options: CsvReadOptions = { delimiter: ",", quote: '"', hasHeader: true, encoding: "utf8" };
const targets = ["first", "last"];
const fileA = csvSource("a.csv", options);
const fileB = csvSource("b.csv", options);
const previewA = { headers: ["first", "last"], rows: [["Ada", "Lovelace"]] };
const previewB = { headers: ["last", "first"], rows: [["Lovelace", "Ada"]] };

describe("CSV import preview", () => {
  it("maps the preview of the current file by its headers", () => {
    const ready = previewFor(settlePreview(loadingPreview(1, fileA), 1, { preview: previewA, targets }), fileA);
    expect(ready?.draft).toEqual(["first", "last"]);
  });

  it("offers no preview or mapping once another file is selected, until that file is read", () => {
    const readyA = settlePreview(loadingPreview(1, fileA), 1, { preview: previewA, targets });
    expect(previewFor(readyA, fileB)).toBeNull();
    const loadingB = loadingPreview(2, fileB);
    expect(previewFor(loadingB, fileB)).toBeNull();
    expect(previewFor(settlePreview(loadingB, 2, { preview: previewB, targets }), fileB)?.draft).toEqual(["last", "first"]);
  });

  it("treats a change of read options as a different source", () => {
    const ready = settlePreview(loadingPreview(1, fileA), 1, { preview: previewA, targets });
    expect(previewFor(ready, csvSource("a.csv", { ...options, delimiter: ";" }))).toBeNull();
    expect(previewFor(ready, csvSource("a.csv", { ...options, hasHeader: false }))).toBeNull();
  });

  it("ignores a response that arrives after a newer read started", () => {
    const loadingB = loadingPreview(2, fileB);
    expect(settlePreview(loadingB, 1, { preview: previewA, targets })).toBe(loadingB);
    expect(settlePreview(loadingB, 1, { error: "gone" })).toBe(loadingB);
    const readyB = settlePreview(loadingB, 2, { preview: previewB, targets });
    expect(settlePreview(readyB, 1, { preview: previewA, targets })).toBe(readyB);
  });

  it("ignores responses after the file is cleared", () => {
    expect(settlePreview({ status: "idle" }, 1, { preview: previewA, targets })).toEqual({ status: "idle" });
  });

  it("keeps a read error for its own source", () => {
    const failed = settlePreview(loadingPreview(1, fileA), 1, { error: "not UTF-8" });
    expect(failed).toEqual({ status: "failed", request: 1, source: fileA, error: "not UTF-8" });
    expect(previewFor(failed, fileA)).toBeNull();
  });
});
