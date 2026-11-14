# OCR golden fixture

- `signs.jpg`: *Tomi Tour Prague* (Zlatnická, Prague, 2009-07-19) by JanSuchy,
  https://commons.wikimedia.org/wiki/File:Tomi_Tour_Prague_2.JPG. **Public domain**
  (`{{PD-self}}` on the file page). The original is 2816 × 2112 (SHA-1
  `ebcc8cc50a148ed3b037c72a82e8f42e6ce47e42`). It was resized here to 1536 × 1152 (Pillow
  Lanczos, JPEG quality 90). It holds an accented line (*cestovní kancelář*), an angled
  facade sign, a road sign and a small logo, so it exercises angled quads, diacritics and
  low confidence.
- `golden.json`: PaddleOCR 3.7.0 (paddlex 3.7.2, paddlepaddle 3.2.2, CPU, no MKL-DNN) with
  PP-OCRv5_mobile_det + latin_PP-OCRv5_mobile_rec, detection at 960 px long side (`max`),
  no document or line-orientation stages, recognition batch 16, every score kept. Written
  by `scripts/ml/export_13.py --golden` on 2026-10-31.

Regenerate both models and the golden with the command in `scripts/ml/export_13.py`'s header.
