"""Train/val/test by a hash of the photo's stem, so a split never changes when data is added
and a RAW, its JPEG and its edited export always share one side."""

import hashlib
import re
from dataclasses import dataclass
from typing import Literal

Split = Literal["train", "val", "test"]

_PREP_HASH = re.compile(r"-[0-9a-f]{16}$")
# What editors add to an export's name: Lightroom's Denoise / Super Resolution / Raw Details
# (`-Enhanced-NR`, `-Enhanced-SR`, `-Enhanced-RS`), merges (`-HDR`, `-Pano`), Photoshop's
# `-Edit`, the Finder's ` copy`, each maybe numbered, and stacked (`-Enhanced-NR-Edit-2`).
# A copy number is never zero-padded: `_01` numbers a sequence of photos, `_2` a copy.
_NUMBER = r"(?:-[1-9]\d?|_[1-9]\d?| [1-9]\d?)"
_ENDINGS = re.compile(rf"(?:(?:-edit|-edited| copy|-enhanced(?:-nr|-sr|-rs)?|-hdr|-pano){_NUMBER}?)+$")
_COPY_NUMBER = re.compile(rf"{_NUMBER}$")


@dataclass(frozen=True)
class Shares:
    """Percent of the whole: `val` validation, `test` held out for good."""

    val: int = 15
    test: int = 2


def stem(name: str) -> str:
    """A file or prepared-folder name → the stem every version of one photo shares."""
    s = name.lower()
    s = re.sub(r"\.[a-z0-9]{1,5}$", "", s)
    s = _PREP_HASH.sub("", s)
    bare = _ENDINGS.sub("", s)
    if bare != s:
        return bare
    # A trailing 1–2 digit number is a copy's only when the photo keeps a number of its own
    # (`img_1234-2`); in `dsc_1` or `img_12` it is the file number itself.
    base = _COPY_NUMBER.sub("", s)
    return base if base[-1:].isdigit() else s


def edited_name(name: str) -> bool:
    """Whether a file's name carries an editor's ending (`-Edit`, `-Enhanced-NR`, ` copy`, …)."""
    s = re.sub(r"\.[a-z0-9]{1,5}$", "", name.lower())
    return bool(_ENDINGS.search(_PREP_HASH.sub("", s)))


def _h(text: str) -> int:
    return int(hashlib.sha1(text.encode("utf-8")).hexdigest(), 16)


def split_of(photo_stem: str, shares: Shares) -> Split:
    if _h("test|" + photo_stem) % 1000 < shares.test * 10:
        return "test"
    scaled = shares.val if shares.test == 0 else round(shares.val * 100 / (100 - shares.test))
    return "val" if _h(photo_stem) % 100 < scaled else "train"
