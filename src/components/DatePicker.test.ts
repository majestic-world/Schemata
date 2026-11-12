import { describe, expect, it } from "vitest";
import { formatPicked, localDate, parseDateText } from "./DatePicker";

describe("date picker values", () => {
  it("keeps years below 100 instead of moving them to 19xx", () => {
    const parsed = parseDateText("0050-03-07");
    expect(parsed?.day.getFullYear()).toBe(50);
    expect(formatPicked("date", parsed!.day, parsed!.time, parsed!.fraction)).toBe("0050-03-07");
    expect(localDate(5, 0, 1).getFullYear()).toBe(5);
    // Month navigation across a year boundary stays in the same century.
    const prev = localDate(50, -1, 1);
    expect([prev.getFullYear(), prev.getMonth()]).toEqual([49, 11]);
  });

  it("opens zero and impossible dates at no day instead of 1899-11-30", () => {
    expect(parseDateText("0000-00-00")).toBeNull();
    expect(parseDateText("0000-00-00 00:00:00")).toBeNull();
    expect(parseDateText("2024-05-00")).toBeNull();
    expect(parseDateText("2023-02-29")).toBeNull();
    expect(parseDateText("not a date")).toBeNull();
  });

  it("keeps the fractional seconds of the value when picking", () => {
    const parsed = parseDateText("2024-02-29 13:45:07.123456")!;
    expect(parsed.time).toBe("13:45:07");
    expect(formatPicked("datetime", localDate(2024, 2, 1), parsed.time, parsed.fraction)).toBe("2024-03-01 13:45:07.123456");
    expect(formatPicked("datetime", parsed.day, "08:30", parsed.fraction)).toBe("2024-02-29 08:30:00.123456");
    expect(formatPicked("datetime", parsed.day, "08:30:00", "")).toBe("2024-02-29 08:30:00");
    expect(formatPicked("date", parsed.day, parsed.time, parsed.fraction)).toBe("2024-02-29");
  });
});
