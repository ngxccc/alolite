import { afterEach, describe, expect, it } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  buildOxfmtArgs,
  findBlockingReleasePr,
  isBreaking,
  isFeature,
  isMajorZero,
  parseInteractiveChoice,
  prependChangelog,
  resolveTarget,
  suggestBump,
} from "./release";

describe("resolveTarget", () => {
  it("takes an explicit version as-is", () => {
    expect(resolveTarget("1.4.2", "1.0.0")).toBe("1.4.2");
  });

  it("bumps relative to the latest tag", () => {
    expect(resolveTarget("patch", "1.4.2")).toBe("1.4.3");
    expect(resolveTarget("minor", "1.4.2")).toBe("1.5.0");
    expect(resolveTarget("major", "1.4.2")).toBe("2.0.0");
  });
});

describe("suggestBump", () => {
  const pr = (number: number, title: string, labels: string[] = []) => ({
    number,
    title,
    mergedAt: "2026-01-01T00:00:00Z",
    labels: labels.map((name) => ({ name })),
  });

  it("has nothing to suggest when nothing merged", () => {
    expect(suggestBump([])).toBeNull();
  });

  it("suggests a patch for fixes and chores", () => {
    expect(suggestBump([pr(1, "fix(chat): bubble"), pr(2, "chore: typos")])).toEqual({
      level: "patch",
      reason: "no feature, no breaking change",
    });
  });

  it("suggests a minor as soon as one feature is present", () => {
    const suggestion = suggestBump([pr(1, "fix(chat): bubble"), pr(2, "feat(ui): tools")]);
    expect(suggestion?.level).toBe("minor");
    expect(suggestion?.reason).toContain("#2");
  });

  it("lets a feature label stand in for a non-conventional title", () => {
    expect(suggestBump([pr(7, "Tools drawer", ["type:feat"])])?.level).toBe("minor");
  });

  it("suggests a major for a breaking change, whatever else merged", () => {
    expect(suggestBump([pr(1, "feat: a"), pr(2, "refactor(core)!: drop legacy")])?.level).toBe(
      "major",
    );
  });

  it("reads the breaking marker after a scope", () => {
    expect(suggestBump([pr(3, "chore(config)!: rename env")])?.level).toBe("major");
  });

  it("caps pre-1.0 breaking changes at minor", () => {
    expect(isMajorZero("0.1.2")).toBe(true);
    expect(isMajorZero("1.0.0")).toBe(false);
    const result = suggestBump([pr(10, "feat(core)!: replace session")], "0.1.2");
    expect(result?.level).toBe("minor");
    expect(result?.reason).toContain("pre-1.0 capped to minor");
  });

  it("detects breaking changes and features from labels", () => {
    expect(isBreaking(pr(4, "refactor: api", ["breaking-change"]))).toBe(true);
    expect(isFeature(pr(5, "New view", ["type:feat"]))).toBe(true);
  });
});

describe("parseInteractiveChoice", () => {
  it("parses confirmation, quick level overrides, and explicit semver", () => {
    expect(parseInteractiveChoice("", "0.2.0", "0.1.2")).toBeNull();
    expect(parseInteractiveChoice("n", "0.2.0", "0.1.2")).toBeNull();
    expect(parseInteractiveChoice("y", "0.2.0", "0.1.2")).toBe("0.2.0");
    expect(parseInteractiveChoice("p", "0.2.0", "0.1.2")).toBe("0.1.3");
    expect(parseInteractiveChoice("m", "0.1.3", "0.1.2")).toBe("0.2.0");
    expect(parseInteractiveChoice("M", "0.2.0", "0.1.2")).toBe("1.0.0");
    expect(parseInteractiveChoice("0.3.0", "0.2.0", "0.1.2")).toBe("0.3.0");
    expect(() => parseInteractiveChoice("invalid", "0.2.0", "0.1.2")).toThrow();
  });
});

describe("findBlockingReleasePr", () => {
  it("detects open release pull requests before branching", () => {
    expect(
      findBlockingReleasePr([{ number: 10, title: "fix(ui): polish", headRefName: "fix/ui" }]),
    ).toBeUndefined();
    expect(
      findBlockingReleasePr([
        { number: 11, title: "chore(release): v0.2.0", headRefName: "chore/release-v0.2.0" },
      ])?.number,
    ).toBe(11);
  });
});

describe("buildOxfmtArgs", () => {
  it("includes --no-error-on-unmatched-pattern for ignored CHANGELOG targets", () => {
    expect(buildOxfmtArgs("CHANGELOG.md", true)).toContain("--no-error-on-unmatched-pattern");
    expect(buildOxfmtArgs("CHANGELOG.md", false)).toEqual([
      "--no-error-on-unmatched-pattern",
      "CHANGELOG.md",
    ]);
  });
});

describe("prependChangelog", () => {
  const dir = mkdtempSync(join(tmpdir(), "release-test-"));
  const path = join(dir, "CHANGELOG.md");

  afterEach(() => {
    rmSync(path, { force: true });
  });

  it("puts the newest version directly under the preamble", () => {
    writeFileSync(
      path,
      "# Changelog\n\nGenerated at release time.\n\n## 1.0.0 - 2026-01-01\n\nold body\n",
    );

    prependChangelog("1.1.0", "### What's Changed\n\n- a thing", path);

    const text = readFileSync(path, "utf8");
    const headings = [...text.matchAll(/^## (.+)$/gm)].map((match) => match[1]);

    expect(headings).toHaveLength(2);
    expect(headings[0]).toMatch(/^1\.1\.0 - \d{4}-\d{2}-\d{2}$/);
    expect(headings[1]).toBe("1.0.0 - 2026-01-01");
    expect(text.startsWith("# Changelog")).toBe(true);
    expect(text.indexOf("### What's Changed")).toBeLessThan(text.indexOf("old body"));
    expect(text.split("# Changelog")).toHaveLength(2);
  });

  it("auto-formats markdown lists and spacing in the generated changelog", () => {
    writeFileSync(
      path,
      "# Changelog\n\nGenerated at release time.\n\n## 1.0.0 - 2026-01-01\n\n*   old bullet\n",
    );

    prependChangelog("1.1.0", "### What's Changed\n*   first item\n*    second item", path);

    const text = readFileSync(path, "utf8");
    expect(text).toContain("### What's Changed\n\n- first item\n- second item");
    expect(text).toContain("## 1.0.0 - 2026-01-01\n\n- old bullet\n");
  });
});
