"""Lumen's model names (Julian, 2026-10-07): a family, a version, and a performance tier.

- **Lumen Ray**: auto-correction (Auto v2, the Retouch model): a quick, direct beam of light.
- **Lumen Prism**: pro editing: the photo broken into colour, curves and masks. Its first model
  (Round 14) is Retouch fitted to Julian's own edits (`prism/`), saved like Ray's.
- **Lumen Spectrum**: library search: seeing across the whole library.

The others (Julian, 2026-10-08) name what the app does with a model, third-party ones included
(credited "based on NAFNet"): **Iris** faces, **Eyeshine** pets, **Spotlight** best shot, **Silk**
noise reduction, **Strobe** music beats, **Glyph** text (OCR), **Facet** AI masks.

Each trained model comes as `low`, `high` and `ultra` (lighter to heavier). A saved model is
`lumen-<family>-<version>-<tier>` (`lumen-ray-1.0-high`), shown as `Lumen Ray-high 1.0`.

Versions: a release is `x.y` (the next release `x.(y+1)`, a major `(x+1).0`, only when Julian says).
Every build before it bumps `x.y.z` and carries its stage: `-d` dev, `-a` alpha, `-b` beta, `-r`
pre-release (the candidate shipped after final checks): `1.0.1-d`, `1.0.2-a`, … `1.0.5-r`."""

import re
from dataclasses import dataclass


@dataclass(frozen=True)
class Family:
    title: str
    purpose: str
    role: str | None  # the app's manifest role, once a family has a model in the app


FAMILIES = {
    "ray": Family("Lumen Ray", "auto-correction", "retouch"),
    "prism": Family("Lumen Prism", "Julian's editing style (Retouch)", "retouch"),
    "spectrum": Family("Lumen Spectrum", "library search", None),
    "iris": Family("Lumen Iris", "faces", None),
    "eyeshine": Family("Lumen Eyeshine", "pets", None),
    "spotlight": Family("Lumen Spotlight", "best shot", None),
    "silk": Family("Lumen Silk", "noise reduction", None),
    "strobe": Family("Lumen Strobe", "music beats", None),
    "glyph": Family("Lumen Glyph", "text in photos", None),
    "facet": Family("Lumen Facet", "AI masks", None),
}
STAGES = {"d": "dev", "a": "alpha", "b": "beta", "r": "pre-release"}
TIERS = ("low", "high", "ultra")
_VERSION = re.compile(r"^\d+\.\d+(\.\d+-[dabr])?$")


def is_release(version: str) -> bool:
    """`2.1` is a release; `1.0.1-d` (any stage) is not."""
    return "-" not in version


def _check(family: str, version: str, tier: str | None) -> None:
    if family not in FAMILIES:
        raise ValueError(f"family must be one of {', '.join(FAMILIES)}, not {family!r}")
    if not _VERSION.match(version):
        raise ValueError(f"version must be a release like 1.1 or a build like 1.0.1-d (stages d, a, b, r), not {version!r}")
    if tier is not None and tier not in TIERS:
        raise ValueError(f"tier must be one of {', '.join(TIERS)}, not {tier!r}")


def model_id(family: str, version: str, tier: str | None = None) -> str:
    _check(family, version, tier)
    return f"lumen-{family}-{version}" + (f"-{tier}" if tier else "")


def display_name(family: str, version: str, tier: str | None = None) -> str:
    _check(family, version, tier)
    return f"{FAMILIES[family].title}{f'-{tier}' if tier else ''} {version}"
