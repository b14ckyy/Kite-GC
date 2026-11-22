#!/usr/bin/env node
// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marc Hoffmann (b14ckyy)
//
// Checks the locale files against en.json, the reference for all string keys.
//
// Usage: node tools/helpers/i18n-check.mjs [--all] [--strict <code>[,<code>...]]
//
//   de.json must match en.json exactly: same keys, same value kinds, same {placeholders}.
//   Every other locale may lack keys (they fall back to English), but must not carry keys
//   that en.json does not have, and a translated string must have the same placeholders.
//   An empty string counts as a missing key: the app shows the English text for it.
//
//   --all      list the missing keys of the best-effort locales instead of only counting them
//   --strict   locales that must be complete; replaces the default (de)
//
// Exit code 1 on any error, 0 otherwise.

import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const LOCALES = join(dirname(fileURLToPath(import.meta.url)), '../../src/lib/i18n/locales');
const REFERENCE = 'en';
// Pseudo-locale written by i18n-key-paths.py
const IGNORED = new Set(['xx']);

const USAGE = 'Usage: node tools/helpers/i18n-check.mjs [--all] [--strict <code>[,<code>...]]';

function usage(message) {
  console.error(message);
  console.error(USAGE);
  process.exit(1);
}

let listAll = false;
let strict = new Set(['de']);
const args = process.argv.slice(2);
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--all') {
    listAll = true;
  } else if (args[i] === '--strict') {
    const value = args[++i];
    if (!value || value.startsWith('--')) usage('--strict needs a locale code');
    strict = new Set(value.split(',').filter(Boolean));
  } else {
    usage(`Unknown argument: ${args[i]}`);
  }
}

function flatten(node, path, out) {
  if (node !== null && typeof node === 'object' && !Array.isArray(node)) {
    for (const [key, value] of Object.entries(node)) flatten(value, path ? `${path}.${key}` : key, out);
  } else {
    out.set(path, node);
  }
  return out;
}

// Approximates the ICU parser: ASCII names only, no '' escapes, no syntax check.
function placeholders(text) {
  const names = new Set();
  let depth = 0;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (ch === "'" && (text[i + 1] === '{' || text[i + 1] === '}')) {
      const end = text.indexOf("'", i + 2);
      i = end < 0 ? text.length : end;
    } else if (ch === '{') {
      const name = /^\s*([A-Za-z_][\w]*)\s*[,}]/.exec(text.slice(i + 1));
      // Odd depth = the text of a plural / select branch, even depth = an argument
      if (name && depth % 2 === 0) names.add(name[1]);
      depth++;
    } else if (ch === '}') {
      depth = Math.max(0, depth - 1);
    }
  }
  return names;
}

function load(code) {
  const file = join(LOCALES, `${code}.json`);
  try {
    return flatten(JSON.parse(readFileSync(file, 'utf8').replace(/^﻿/, '')), '', new Map());
  } catch (err) {
    console.error(`${code}.json: ${err.message}`);
    process.exit(1);
  }
}

const reference = load(REFERENCE);
const codes = readdirSync(LOCALES)
  .filter((f) => f.endsWith('.json'))
  .map((f) => f.slice(0, -5))
  .filter((c) => c !== REFERENCE && !IGNORED.has(c))
  .sort((a, b) => Number(strict.has(b)) - Number(strict.has(a)) || a.localeCompare(b));

for (const code of strict) {
  if (!codes.includes(code)) usage(`--strict: ${code} is not a locale that can be checked`);
}

let errors = 0;
console.log(`${REFERENCE}.json: ${reference.size} keys (reference)`);

for (const code of codes) {
  const locale = load(code);
  const isStrict = strict.has(code);
  const missing = [];
  const problems = [];

  for (const [key, refValue] of reference) {
    if (!locale.has(key) || (locale.get(key) === '' && refValue !== '')) {
      missing.push(key);
      continue;
    }
    const value = locale.get(key);
    if (typeof value !== typeof refValue) {
      problems.push(`${key}: ${typeof value} instead of ${typeof refValue}`);
    } else if (typeof value === 'string') {
      const want = placeholders(refValue);
      const have = placeholders(value);
      const lost = [...want].filter((n) => !have.has(n));
      const added = [...have].filter((n) => !want.has(n));
      if (lost.length || added.length) {
        const parts = [];
        if (lost.length) parts.push(`lacks {${lost.join('}, {')}}`);
        if (added.length) parts.push(`has unknown {${added.join('}, {')}}`);
        problems.push(`${key}: ${parts.join(', ')}`);
      }
    }
  }
  for (const key of locale.keys()) {
    if (!reference.has(key)) problems.push(`${key}: not in ${REFERENCE}.json`);
  }

  const failed = problems.length + (isStrict ? missing.length : 0);
  errors += failed;
  const state = failed ? 'FAIL' : 'ok';
  const gap = missing.length ? `, ${missing.length} missing${isStrict ? '' : ' (fall back to English)'}` : '';
  console.log(`${code}.json: ${state}, ${locale.size} keys${gap}${isStrict ? ' [strict]' : ''}`);

  if (isStrict || listAll) for (const key of missing) console.log(`  missing: ${key}`);
  for (const line of problems) console.log(`  ${line}`);
}

if (errors) {
  console.error(`\n${errors} error(s)`);
  process.exitCode = 1;
}
