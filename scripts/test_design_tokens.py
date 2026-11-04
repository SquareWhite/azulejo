#!/usr/bin/env python3
"""Tests for scripts/design-tokens: the themes in shared/design/tokens.json.

    python3 scripts/test_design_tokens.py     (run by `just lint`)
"""

from __future__ import annotations

import copy
import importlib.machinery
import importlib.util
import json
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
loader = importlib.machinery.SourceFileLoader("design_tokens", str(ROOT / "scripts" / "design-tokens"))
spec = importlib.util.spec_from_loader("design_tokens", loader)
gen = importlib.util.module_from_spec(spec)
loader.exec_module(gen)

TOKENS = json.loads((ROOT / "shared" / "design" / "tokens.json").read_text())
BUILT_INS = ["atom-one-dark", "atom-one-light", "macos-dark", "macos-light"]


def names() -> list[str]:
    return [k for k in TOKENS["colorTokens"] if not k.startswith("_")]


class Themes(unittest.TestCase):
    def test_the_file_validates(self):
        self.assertEqual(gen.validate(TOKENS), [])

    def test_four_built_ins_define_every_colour_token(self):
        themes = [k for k in TOKENS["themes"] if not k.startswith("_")]
        self.assertEqual(themes, BUILT_INS)
        for tid in BUILT_INS:
            colours = {k for k in TOKENS["themes"][tid]["color"] if not k.startswith("_")}
            self.assertEqual(colours, set(names()), tid)
        self.assertEqual(TOKENS["defaultTheme"], "atom-one-dark")

    def test_appearances(self):
        got = {tid: TOKENS["themes"][tid]["appearance"] for tid in BUILT_INS}
        self.assertEqual(got, {"atom-one-dark": "dark", "atom-one-light": "light",
                               "macos-dark": "dark", "macos-light": "light"})

    def test_a_missing_token_fails(self):
        broken = copy.deepcopy(TOKENS)
        del broken["themes"]["macos-light"]["color"]["accent"]
        self.assertIn("themes.macos-light.color: missing accent", gen.validate(broken))

    def test_an_extra_token_fails(self):
        broken = copy.deepcopy(TOKENS)
        broken["themes"]["atom-one-light"]["color"]["sparkle"] = "#ffffff"
        self.assertIn("themes.atom-one-light.color.sparkle: not in colorTokens", gen.validate(broken))

    def test_bad_values_fail(self):
        broken = copy.deepcopy(TOKENS)
        theme = broken["themes"]["macos-dark"]
        theme["color"]["window"] = "blue"
        theme["color"]["tile"] = {"system": "notAColor", "hex": "#000000"}
        theme["color"]["accent"] = {"system": "controlAccentColor"}
        theme["appearance"] = "sepia"
        theme["terminal"]["palette"] = theme["terminal"]["palette"][:15]
        problems = "\n".join(gen.validate(broken))
        for needle in ["color.window: 'blue'", "unknown system colour 'notAColor'",
                       "color.accent: needs \"hex\"", "appearance must be light or dark",
                       "terminal.palette: needs 16"]:
            self.assertIn(needle, problems)

    def test_generated_swift_carries_every_theme(self):
        swift = gen.generate(TOKENS)
        for tid in BUILT_INS:
            self.assertIn(f'id: "{tid}"', swift)
        for name in names():
            self.assertIn(f'static let {name} = Theme.token("{name}")', swift)
        # atom-one-dark is today's look: the mockup's window colour.
        self.assertIn('"window": .rgba(0x1b1d23ff)', swift)
        self.assertIn('.system("controlAccentColor"', swift)

    def test_design_swift_is_fresh(self):
        self.assertEqual(gen.OUT.read_text(), gen.generate(TOKENS),
                         "Design.swift is stale: run scripts/design-tokens")


if __name__ == "__main__":
    sys.exit(0 if unittest.main(exit=False, verbosity=1).result.wasSuccessful() else 1)
