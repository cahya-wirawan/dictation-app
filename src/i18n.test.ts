import { describe, expect, it } from "vitest";
import { t, tp } from "./i18n";

describe("platform message variants", () => {
  it("uses a platform variant when it exists and falls back otherwise", () => {
    expect(tp("onb.paste.body", "windows")).toContain("Ctrl+V");
    expect(tp("onb.paste.body", "macos")).toBe(t("onb.paste.body"));
    expect(tp("onb.paste.body", "linux")).toBe(t("onb.paste.body"));
    expect(tp("settings.openSettings", "windows")).toBe("Open Windows Settings");
  });
});
