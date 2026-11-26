import { describe, expect, it } from "bun:test";
import fs from "node:fs";
import path from "node:path";
import ts from "typescript";

const DESKTOP_ROOT = path.resolve(import.meta.dir, "..");
const SRC_ROOT = "src";
const SHARED_UI_PREFIX = "src/shared/ui/";

const FORBIDDEN_RAW_ELEMENTS = new Set([
  "table",
  "thead",
  "tbody",
  "tfoot",
  "tr",
  "th",
  "td",
  "caption",
  "textarea",
  "button",
  "input",
  "select",
]);

const FORBIDDEN_NESTED_IN_LABEL = new Set([
  "input",
  "Input",
  "button",
  "Button",
  "select",
  "Select",
  "SelectTrigger",
  "textarea",
  "Textarea",
  "a",
  "Link",
]);

function collectSourceFiles(rootDir: string): string[] {
  const results: string[] = [];
  if (!fs.existsSync(rootDir)) return results;

  for (const entry of fs.readdirSync(rootDir, { withFileTypes: true })) {
    const fullPath = path.join(rootDir, entry.name);
    if (entry.isDirectory()) {
      results.push(...collectSourceFiles(fullPath));
    } else if (entry.isFile() && /\.(?:ts|tsx)$/.test(entry.name)) {
      results.push(fullPath);
    }
  }

  return results.sort();
}

function parseSourceFile(filePath: string, sourceText: string): ts.SourceFile {
  const scriptKind = filePath.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS;
  return ts.createSourceFile(filePath, sourceText, ts.ScriptTarget.Latest, true, scriptKind);
}

function hasHtmlForAttribute(attributes: ts.JsxAttributes): boolean {
  return attributes.properties.some(
    (attr) => ts.isJsxAttribute(attr) && ts.isIdentifier(attr.name) && attr.name.text === "htmlFor",
  );
}

function containsNestedDisallowedTag(
  root: ts.Node,
  labelLine: number,
  sourceFile: ts.SourceFile,
): string | null {
  let nestedTag: string | null = null;

  function visit(node: ts.Node): void {
    if (nestedTag) return;
    if (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) {
      const tagName = node.tagName.getText(sourceFile);
      if (FORBIDDEN_NESTED_IN_LABEL.has(tagName)) {
        nestedTag = `${tagName} at line ${labelLine}`;
        return;
      }
    }
    ts.forEachChild(node, visit);
  }

  ts.forEachChild(root, visit);
  return nestedTag;
}

/**
 * Inspects a TypeScript or TSX source string for raw `<label>` JSX elements, missing
 * `htmlFor` attributes on shared `<Label>` usages, and interactive controls nested inside `<Label>`.
 */
export function evaluateUiLabelHygiene(filePath: string, sourceText: string): string[] {
  const sourceFile = parseSourceFile(filePath, sourceText);
  const violations: string[] = [];

  function visit(node: ts.Node): void {
    if (ts.isJsxSelfClosingElement(node)) {
      const tagName = node.tagName.getText(sourceFile);
      const line = sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line + 1;
      if (tagName === "label") {
        violations.push(`raw label element at line ${line}`);
      } else if (tagName === "Label" && !hasHtmlForAttribute(node.attributes)) {
        violations.push(`Label missing htmlFor at line ${line}`);
      }
    } else if (ts.isJsxElement(node)) {
      const opening = node.openingElement;
      const tagName = opening.tagName.getText(sourceFile);
      const line = sourceFile.getLineAndCharacterOfPosition(opening.getStart(sourceFile)).line + 1;

      if (tagName === "label") {
        violations.push(`raw label element at line ${line}`);
      } else if (tagName === "Label") {
        if (!hasHtmlForAttribute(opening.attributes)) {
          violations.push(`Label missing htmlFor at line ${line}`);
        }
        const nestedViolation = containsNestedDisallowedTag(node, line, sourceFile);
        if (nestedViolation) {
          violations.push(`nested ${nestedViolation} inside Label`);
        }
      }
    }

    ts.forEachChild(node, visit);
  }

  visit(sourceFile);
  return violations;
}

/**
 * Inspects a TypeScript or TSX source string for raw HTML table, button, input, select,
 * and textarea elements that bypass shadcn `src/shared/ui/*` primitives.
 */
export function evaluatePrimitiveHygiene(filePath: string, sourceText: string): string[] {
  const sourceFile = parseSourceFile(filePath, sourceText);
  const violations: string[] = [];

  function visit(node: ts.Node): void {
    if (ts.isJsxSelfClosingElement(node) || ts.isJsxOpeningElement(node)) {
      const tagName = node.tagName.getText(sourceFile);
      if (FORBIDDEN_RAW_ELEMENTS.has(tagName)) {
        const line = sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line + 1;
        violations.push(`raw ${tagName} element at line ${line}`);
      }
    }
    ts.forEachChild(node, visit);
  }

  visit(sourceFile);
  return violations;
}

/**
 * Rejects heavy DOM mount test imports (`happy-dom`, `react-dom/client`) in unit tests.
 */
export function evaluateDomMountTestHygiene(relativePath: string, sourceText: string): string[] {
  if (
    !relativePath.endsWith(".spec.ts") &&
    !relativePath.endsWith(".spec.tsx") &&
    !relativePath.endsWith(".test.ts") &&
    !relativePath.endsWith(".test.tsx")
  ) {
    return [];
  }

  const sourceFile = parseSourceFile(relativePath, sourceText);
  const violations: string[] = [];

  for (const statement of sourceFile.statements) {
    if (ts.isImportDeclaration(statement) && ts.isStringLiteral(statement.moduleSpecifier)) {
      const specifier = statement.moduleSpecifier.text;
      if (specifier === "happy-dom" || specifier === "react-dom/client") {
        const line = sourceFile.getLineAndCharacterOfPosition(statement.getStart()).line + 1;
        violations.push(
          `Forbidden DOM mount import "${specifier}" at line ${line} (unit-test pure logic/stores only)`,
        );
      }
    }
  }

  return violations;
}

function toPosixRelative(file: string): string {
  return path.relative(DESKTOP_ROOT, file).split(path.sep).join("/");
}

describe("Desktop UI Primitives Hygiene", () => {
  describe("evaluateUiLabelHygiene", () => {
    it("rejects raw lowercase label tags, unbound Label elements, and nested controls", () => {
      const rawTagResult = evaluateUiLabelHygiene(
        "sample.tsx",
        `<label htmlFor="q">Search</label>`,
      );
      const unboundWrapperResult = evaluateUiLabelHygiene(
        "sample.tsx",
        `<Label className="flex"><Input id="q" /><Button type="submit">Go</Button></Label>`,
      );

      expect(rawTagResult).toEqual(["raw label element at line 1"]);
      expect(unboundWrapperResult).toEqual([
        "Label missing htmlFor at line 1",
        "nested Input at line 1 inside Label",
      ]);
    });

    it("allows shared Label bound via htmlFor with Checkbox and external Input controls", () => {
      const violations = evaluateUiLabelHygiene(
        "valid.tsx",
        `<>
          <Label htmlFor="q" className="sr-only">Search</Label>
          <div><Input id="q" /><Button type="submit">Go</Button></div>
          <Label htmlFor="agree"><Checkbox id="agree" /><span>Accept</span></Label>
        </>`,
      );

      expect(violations).toEqual([]);
    });
  });

  describe("evaluatePrimitiveHygiene", () => {
    it("rejects raw table, textarea, input, select, and button tags while ignoring JSX components", () => {
      const violations = evaluatePrimitiveHygiene(
        "sample.tsx",
        `<div>
          <Input placeholder="OK" />
          <table><tbody><tr><td>Bad</td></tr></tbody></table>
          <textarea rows={3} />
          <button type="button">Raw</button>
        </div>`,
      );

      expect(violations).toEqual([
        "raw table element at line 3",
        "raw tbody element at line 3",
        "raw tr element at line 3",
        "raw td element at line 3",
        "raw textarea element at line 4",
        "raw button element at line 5",
      ]);
    });
  });

  describe("desktop UI sources", () => {
    it("rejects raw label tags and nested wrapper Labels across feature sources", () => {
      const violations: string[] = [];

      for (const file of collectSourceFiles(path.join(DESKTOP_ROOT, SRC_ROOT))) {
        const rel = toPosixRelative(file);
        if (rel.startsWith(SHARED_UI_PREFIX)) continue;

        const fileViolations = evaluateUiLabelHygiene(file, fs.readFileSync(file, "utf8"));
        for (const violation of fileViolations) {
          violations.push(`${rel}: ${violation}`);
        }
      }

      expect(violations).toEqual([]);
    });

    it("rejects raw HTML button, input, select, textarea, and table tags outside src/shared/ui", () => {
      const violations: string[] = [];

      for (const file of collectSourceFiles(path.join(DESKTOP_ROOT, SRC_ROOT))) {
        const rel = toPosixRelative(file);
        if (rel.startsWith(SHARED_UI_PREFIX)) continue;

        const fileViolations = evaluatePrimitiveHygiene(file, fs.readFileSync(file, "utf8"));
        for (const violation of fileViolations) {
          violations.push(`${rel}: ${violation}`);
        }
      }

      expect(violations).toEqual([]);
    });

    it("rejects happy-dom or react-dom/client mount imports in unit tests", () => {
      const violations: string[] = [];

      for (const file of collectSourceFiles(path.join(DESKTOP_ROOT, SRC_ROOT))) {
        const rel = toPosixRelative(file);
        const fileViolations = evaluateDomMountTestHygiene(rel, fs.readFileSync(file, "utf8"));
        for (const violation of fileViolations) {
          violations.push(`${rel}: ${violation}`);
        }
      }

      expect(violations).toEqual([]);
    });
  });
});
