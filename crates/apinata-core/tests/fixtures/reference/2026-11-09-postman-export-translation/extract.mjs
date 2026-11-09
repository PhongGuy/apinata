// Vendors the reference's script translation tests and tables as JSON.
// Usage: node extract.mjs <bruno checkout> <output dir>
// Each test file runs with stand-in test globals: the translator records the case's input,
// and `expect(...)` records each assertion's kind and text instead of checking it.
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const [repo, out] = process.argv.slice(2);
const pkg = join(repo, 'packages/bruno-converters');
const dir = join(pkg, 'tests/bruno/bruno-to-postman-translations');
const cases = [];
for (const file of readdirSync(dir).filter((f) => f.endsWith('.test.js')).sort()) {
  const group = file.replace('.test.js', '');
  const body = readFileSync(join(dir, file), 'utf8').replace(/^import .*$/m, '');
  let current = null;
  const translate = (input) => {
    current.input = input;
    const output = { output: true, trim: () => output };
    return output;
  };
  const expect = (value) => {
    const record = (kind) => (text) => {
      if (!value || value.output !== true || typeof text !== 'string') {
        throw new Error(`${current.id}: unsupported assertion`);
      }
      current.checks.push({ kind, text });
    };
    return { toBe: record('toBe'), toContain: record('toContain') };
  };
  let n = 0;
  const it = (name, fn) => {
    n += 1;
    current = { id: `${group}/${String(n).padStart(2, '0')}`, name, input: null, checks: [] };
    fn();
    if (current.input === null || current.checks.length === 0) {
      throw new Error(`${current.id}: no input or no assertion`);
    }
    cases.push(current);
  };
  const describe = (_name, fn) => fn();
  new Function('describe', 'it', 'test', 'expect', 'translateBruToPostman', body)(
    describe, it, it, expect, translate
  );
}

// The one-to-one table is an object literal; the call-to-property rows are the complex
// transformations whose transform only builds a member expression.
const source = readFileSync(join(pkg, 'src/utils/bruno-to-postman-translator.js'), 'utf8');
const simple = eval(`(${source.match(/const simpleTranslations = (\{[\s\S]*?\n\});/)[1]})`);
const callToProperty = {};
const row = /pattern: '([^']+)',\s*transform: \(\) => \{?\s*(?:\/\/[^\n]*\n\s*)?(?:return )?buildMemberExpressionFromString\('([^']+)'\)/g;
for (const m of source.matchAll(row)) callToProperty[m[1]] = m[2];

writeFileSync(join(out, 'cases.json'), `${JSON.stringify(cases, null, 2)}\n`);
writeFileSync(join(out, 'table.json'), `${JSON.stringify({ simple, callToProperty }, null, 2)}\n`);
console.log(`${cases.length} cases, ${Object.keys(simple).length} simple rows, ${Object.keys(callToProperty).length} call-to-property rows`);
