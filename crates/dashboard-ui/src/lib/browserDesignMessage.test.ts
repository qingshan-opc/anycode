import { describe, expect, it } from "vitest";
import {
  chipLabel,
  encodeBrowserDesignMessage,
  parseBrowserDesignMessage,
} from "./browserDesignMessage";

describe("browserDesignMessage", () => {
  const hit = {
    tag: "ntp-app",
    css_selector: "body > ntp-app",
    xpath: "/body[1]/ntp-app[1]",
    url: "chrome://new-tab-page/",
  };

  it("chipLabel prefers short selector", () => {
    expect(chipLabel(hit)).toBe("body > ntp-app");
    expect(chipLabel({ ...hit, id: "main" })).toBe("ntp-app#main");
  });

  it("round-trips encode/parse for display", () => {
    const encoded = encodeBrowserDesignMessage(hit, "make it darker");
    expect(encoded).toContain("@@browser-design@@");
    const parsed = parseBrowserDesignMessage(encoded);
    expect(parsed?.hit.css_selector).toBe("body > ntp-app");
    expect(parsed?.instruction).toBe("make it darker");
  });
});
