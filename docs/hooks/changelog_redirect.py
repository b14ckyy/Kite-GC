# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Marc Hoffmann (b14ckyy)
"""mkdocs hook: one changelog for every released line.

The docs are versioned per release line — `release/<x.y>.x` deploys the docs version "<x.y>", see
.github/workflows/docs.yml — and every release branch carries the whole changelog.md: patch notes are
written once, on the line they belong to, and reach the newer lines with the merge-up. So the newest
release line always holds the complete changelog and the older lines need no copy of their own. With
KITE_DOCS_VERSION (the version being built) and KITE_DOCS_CHANGELOG_HOME (the newest line) set to
different values, this hook replaces the rendered changelog page with a redirect to the newest line's
page. The workflow sets both for every release deploy; the Dev deploy and a local `mkdocs serve` run
without them and render the branch's own changelog, in-development box included.

Whichever branch renders it, the NEWEST release box is the expanded one: the hook opens the first
top-level box and folds the others, so the markers in the file do not matter and a merge-up carries no
per-line "which box is open" edits.
"""

import os
import re

SECTION_RE = re.compile(r'^\?\?\?\+? \w+ "')

REDIRECT = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta http-equiv="refresh" content="0; url={target}">
<link rel="canonical" href="{target}">
<title>Changelog — Kite Ground Control</title>
</head>
<body>
<p>The changelog is kept once for every version — <a href="{target}">open it</a>.</p>
</body>
</html>
"""


def on_page_markdown(markdown, page, config, files):
    if page.file.src_uri != "changelog.md":
        return markdown
    out = []
    first = True
    for line in markdown.splitlines(keepends=True):
        if SECTION_RE.match(line):
            line = ("???+" if first else "???") + line[line.index(" ") :]
            first = False
        out.append(line)
    return "".join(out)


def on_post_page(output, page, config):
    if page.file.src_uri != "changelog.md":
        return output
    home = os.environ.get("KITE_DOCS_CHANGELOG_HOME", "").strip()
    mine = os.environ.get("KITE_DOCS_VERSION", "").strip()
    if not home or not mine or home == mine:
        return output
    # The page lives at <site>/<version>/changelog/; the newest line's copy is a sibling version folder.
    return REDIRECT.format(target=f"../../{home}/changelog/")
