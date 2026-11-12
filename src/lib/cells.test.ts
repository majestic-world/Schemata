import { describe, expect, it } from "vitest";
import { editedCell, editorKeyAction, editorText } from "./cells";

const keys = { shiftKey: false, altKey: false, ctrlKey: false, metaKey: false, isComposing: false };

describe("cell editor values", () => {
  it("opening and closing the editor without typing is not an edit, whatever the line breaks", () => {
    for (const value of ["a\nb", "a\r\nb", "a\rb", "", "plain"]) {
      expect(editedCell(value, null)).toBeNull();
      // A text control reports the same text with its line breaks normalized to \n.
      expect(editedCell(value, value.replace(/\r\n?/g, "\n"))).toBeNull();
    }
    expect(editedCell(editorText({ type: "null" }), "")).toBeNull();
  });

  it("keeps line breaks of edited text", () => {
    expect(editedCell(editorText({ type: "text", value: "a" }), "a\nb")).toEqual({ type: "text", value: "a\nb" });
    expect(editedCell("line1\nline2", "line1\nline2!")).toEqual({ type: "text", value: "line1\nline2!" });
  });

  it("keeps the \\r\\n line breaks of a value that used them", () => {
    expect(editedCell("a\r\nb", "a\nb\nc")).toEqual({ type: "text", value: "a\r\nb\r\nc" });
  });

  it("starts NULL and binary cells empty", () => {
    expect(editorText({ type: "null" })).toBe("");
    expect(editorText({ type: "bytes", value: { previewHex: "00", len: 1 } })).toBe("");
    expect(editorText({ type: "int", value: 5 })).toBe("5");
  });
});

describe("cell editor keys", () => {
  it("Enter saves, Shift+Enter and Alt+Enter insert a line break", () => {
    expect(editorKeyAction({ ...keys, key: "Enter" }, false)).toBe("commit");
    expect(editorKeyAction({ ...keys, key: "Enter", shiftKey: true }, false)).toBe("newline");
    expect(editorKeyAction({ ...keys, key: "Enter", altKey: true }, false)).toBe("newline");
    expect(editorKeyAction({ ...keys, key: "Enter", isComposing: true }, false)).toBeNull();
  });

  it("Tab saves, Esc cancels, Alt+Down opens the picker only when there is one", () => {
    expect(editorKeyAction({ ...keys, key: "Tab" }, false)).toBe("commit");
    expect(editorKeyAction({ ...keys, key: "Escape" }, false)).toBe("cancel");
    expect(editorKeyAction({ ...keys, key: "ArrowDown", altKey: true }, true)).toBe("picker");
    expect(editorKeyAction({ ...keys, key: "ArrowDown", altKey: true }, false)).toBeNull();
  });
});
