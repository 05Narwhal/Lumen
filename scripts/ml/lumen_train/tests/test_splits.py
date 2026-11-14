"""Stems and splits: renamed exports share a side, and growing the data moves nothing."""

import hashlib
from collections import Counter

import pytest

from lumen_train.data import splits


@pytest.mark.parametrize("name", ["IMG_1234.CR3", "IMG_1234.jpg", "IMG_1234-Edit.jpg", "IMG_1234 copy.jpg", "img_1234-2.JPG", "IMG_1234-a1b2c3d4e5f60718"])
def test_one_stem(name):
    assert splits.stem(name) == "img_1234"


@pytest.mark.parametrize(
    "name",
    [
        "IMG_1234-Enhanced-NR.jpg",  # Lightroom Denoise
        "IMG_1234-Enhanced-SR.dng",  # Super Resolution
        "IMG_1234-Enhanced-RS.dng",  # Raw Details
        "IMG_1234-Enhanced.dng",
        "IMG_1234-HDR.dng",
        "IMG_1234-Pano.jpg",
        "IMG_1234-Enhanced-NR-Edit.jpg",  # stacked: Denoise, then Photoshop
        "IMG_1234-Edit-2.png",
        "IMG_1234 copy 2.tif",
        "IMG_1234-Enhanced-NR-a1b2c3d4e5f60718",  # its prepared folder
    ],
)
def test_lightroom_and_photoshop_endings(name):
    assert splits.stem(name) == "img_1234"


def test_a_number_in_the_name_is_kept():
    """Only one trailing copy number goes; a name made of numbers keeps the rest."""
    assert splits.stem("IMG_20_1.jpg") == "img_20"
    assert splits.stem("DSC_0134.NEF") == "dsc_0134"
    assert splits.stem("Hdr-sunset.jpg") == "hdr-sunset"


def test_splits_stable_when_data_grows():
    shares = splits.Shares(val=12, test=18)
    before = {f"p{i}": splits.split_of(f"p{i}", shares) for i in range(1000)}
    after = {f"p{i}": splits.split_of(f"p{i}", shares) for i in range(2000)}
    assert all(after[k] == v for k, v in before.items())


def test_shares_are_respected():
    got = Counter(splits.split_of(f"s{i}", splits.Shares(val=12, test=18)) for i in range(20000))
    assert abs(got["test"] / 20000 - 0.18) < 0.01 and abs(got["val"] / 20000 - 0.12) < 0.01


def test_default_val_matches_r1():
    # r1's rule: sha1(stem) % 100 < 15 → val. With test=0 the new rule must agree exactly.
    for i in range(500):
        s = f"photo{i}"
        old = int(hashlib.sha1(s.encode()).hexdigest(), 16) % 100 < 15
        assert (splits.split_of(s, splits.Shares(val=15, test=0)) == "val") == old


@pytest.mark.parametrize("raw, edit", [("DSC_1.NEF", "DSC_1-Edit.jpg"), ("IMG_12.CR2", "IMG_12-Edit.jpg"), ("IMG_12.CR2", "IMG_12 copy.jpg")])
def test_a_short_file_number_is_the_photos_own(raw, edit):
    """Review 1: a 1–2 digit file number is not a copy number, or the RAW and its export part."""
    assert splits.stem(raw) == splits.stem(edit) == raw.lower().rsplit(".", 1)[0]


def test_short_numbered_photos_stay_apart():
    stems = {splits.stem(n) for n in ["IMG_5.jpg", "IMG_12.jpg", "photo-3.jpg", "photo-4.jpg", "IMG_1.CR3"]}
    assert len(stems) == 5


@pytest.mark.parametrize("names", [["20231007_01.NEF", "20231007_02.NEF", "20231007_09.NEF"], ["IMG_20231007_02.DNG", "IMG_20231007_03.DNG"]])
def test_sequence_numbers_are_not_copy_numbers(names):
    """A zero-padded `_01` numbers a sequence of photos; a copy is `-2`, `_2` or ` 2`."""
    assert len({splits.stem(n) for n in names}) == len(names)
    assert splits.stem("20231007_01-Edit.jpg") == splits.stem("20231007_01.NEF")
