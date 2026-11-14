"""Build src-tauri/resources/localities.tsv.gz from GeoNames' cities500 (Round 4K).

Every populated place of 500 people or more (~236k), so a photo taken in a village such as
Kropa is named by the village, not by the nearest town of 15,000. Same columns as
cities.tsv.gz: name, country code, latitude, longitude, population.

    curl -C - -LO https://download.geonames.org/export/dump/cities500.zip
    unzip cities500.zip
    python scripts/build-localities.py cities500.txt

GeoNames data is CC BY 4.0 (https://www.geonames.org/).
"""

import gzip
import pathlib
import sys

OUT = pathlib.Path(__file__).resolve().parent.parent / "src-tauri" / "resources" / "localities.tsv.gz"


def main(src: str) -> None:
    rows = []
    with open(src, encoding="utf-8") as f:
        for line in f:
            c = line.rstrip("\n").split("\t")
            # 1 name, 4 latitude, 5 longitude, 8 country code, 14 population
            name, lat, lon, cc, pop = c[1], c[4], c[5], c[8], c[14] or "0"
            if not name or not cc:
                continue
            rows.append(f"{name}\t{cc}\t{float(lat):.4f}\t{float(lon):.4f}\t{pop}")
    rows.sort()
    with gzip.open(OUT, "wt", encoding="utf-8", compresslevel=9) as g:
        g.write("\n".join(rows) + "\n")
    print(f"{len(rows)} places -> {OUT} ({OUT.stat().st_size / 1e6:.1f} MB)")


if __name__ == "__main__":
    main(sys.argv[1])
