import { describe, expect, it } from "vitest";
import {
  consumeComposerInject,
  injectComposerText,
} from "./composerInject";

describe("composerInject", () => {
  it("queues and consumes once", () => {
    injectComposerText({ text: "hello", focus: true, source: "test" });
    expect(consumeComposerInject()?.text).toBe("hello");
    expect(consumeComposerInject()).toBeNull();
  });
});
