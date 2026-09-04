# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)
"""mkdocs hook: hide unreleased changelog sections on the release docs channel.

The docs workflow deploys BOTH the current minor ("1.0" = latest) and "Dev" from the same master
tip, so without this the release version would show the "x.y.z — in development" section too.
With KITE_DOCS_CHANNEL=release (set only for the minor deploy step in .github/workflows/docs.yml)
this hook drops every release section of changelog.md whose title says "in development", then
expands the first remaining section so the "current version expanded, older folded" rule holds.
The Dev deploy and a local `mkdocs serve` run without the variable and show everything.
"""

import os
import re

UNRELEASED_MARKER = "in development"
SECTION_RE = re.compile(r'^\?\?\?\+? \w+ "([^"]*)"')


def on_page_markdown(markdown, page, config, files):
    if page.file.src_uri != "changelog.md":
        return markdown
    if os.environ.get("KITE_DOCS_CHANNEL", "").strip().lower() != "release":
        return markdown

    out = []
    skipping = False
    for line in markdown.splitlines(keepends=True):
        match = SECTION_RE.match(line)
        if match:
            skipping = UNRELEASED_MARKER in match.group(1).lower()
            if skipping:
                continue
        elif skipping:
            # A section body is its indented (or blank) lines; the first flush-left, non-blank
            # line — the next section, or the link references at the end — ends it.
            if line.strip() == "" or line.startswith("    "):
                continue
            skipping = False
        out.append(line)

    # The newest remaining release is the one this docs version describes → expanded.
    for i, line in enumerate(out):
        if line.startswith("???+ "):
            break
        if line.startswith("??? "):
            out[i] = "???+ " + line[4:]
            break
    return "".join(out)
