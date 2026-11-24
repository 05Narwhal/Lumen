# Third-party models

Lumen fetches these files on first launch (ML design §2.3) into `config_dir/JL-Studios/Lumen/models/`.
They are produced by `scripts/ml/export_models.py`; the manifest is `src-tauri/crates/photos-models/models.json`.

## The rule (2026-11-14)

Lumen is free GPL software, so the rule is about training data and the weights' licence, not about a paid product:

- A model may learn from non-commercial (NC) sources if its weights are then published as **CC-BY-NC-4.0** and listed under *Non-commercial weights* below. Its `models.json` `licence` string starts with `CC-BY-NC-4.0`.
- A model published under any other licence (MIT, Apache-2.0) learns only from sources that allow commercial use. `scripts/ml/lumen_train/common/licence.py` refuses a `ship` run that mixes the two.
- NC-trained weights are never shipped under a permissive licence.
- Older entries below that say "commercial use" or "§2.8" were vetted under the previous rule and stay valid under the stricter one.

## Non-commercial weights (CC-BY-NC-4.0)

### Lumen Wayfinder 1.0 — `wayfinder-image-fp16.onnx`, `wayfinder-location.onnx`, `wayfinder-gallery.bin`

- **What:** GeoCLIP (Vivanco, Nayak, Jha and Shah, *GeoCLIP: Clip-Inspired Alignment between Locations and Images for Effective Worldwide Geo-localization*, NeurIPS 2023), exported to ONNX by `scripts/ml/export_wayfinder.py` (opset 17). Nothing is trained or fine-tuned: the files are GeoCLIP's own weights in another format.
  - `wayfinder-image-fp16.onnx`: CLIP ViT-L/14 vision tower + visual projection + GeoCLIP's image MLP, L2-normalised 512-d output, weights fp16 with fp32 input and output.
  - `wayfinder-location.onnx`: GeoCLIP's location encoder (equal-earth projection, three random-Fourier-feature capsules), fp32, unit-length 512-d output.
  - `wayfinder-gallery.bin`: the location encoder run over GeoCLIP's 100,000-coordinate gallery once (header `LWVG`, version 1, rows, dim, dtype 1 = f16; f16 matrix; f32 lat/lon rows). A `kind = "vectors"` model: the host ranks it (`models.nearest`).
- **Source:** `geoclip` 1.3.0 on PyPI (https://github.com/VicenteVivan/geo-clip, code and weights MIT) and OpenAI's `openai/clip-vit-large-patch14` (MIT). Checkpoint SHA-256: `image_encoder_mlp_weights.pth` 5342c391eec961796d04d02f2bd670bfa97b34ce32c656d0e0676581103c2c14, `location_encoder_weights.pth` 94b80ae3af89fca78539e129dd2929d321247f5ceeeb0a31bc2c31041a253394, `logit_scale_weights.pth` 9568e2e09dd19083082bf285b2c0c126867e40dbd275a8ee8c621888b861904e, `coordinates_100K.csv` 3a00b1ba58f13443db726533b6399b39609e292df09d4e58792fa5949d1838b1, CLIP `model.safetensors` a2bf730a0c7debf160f7a6b50b3aaf3703e7e88ac73de7a314903141db026dcb.
- **Licence:** `CC-BY-NC-4.0`. The code and weights of GeoCLIP and CLIP are MIT, but GeoCLIP was trained on MP-16 (Flickr photos under mixed licences, some non-commercial), so Lumen publishes the exported files under the stricter rule above and credits GeoCLIP and its authors in the plugin's README. `models.json` / the plugin manifest `licence` string starts with `CC-BY-NC-4.0`.
- **Files (2026-11-21 export):**

  | File | Bytes | SHA-256 | BLAKE3 |
  |---|---|---|---|
  | `wayfinder-image-fp16.onnx` | 610,526,312 | 28c5503f2871b3f65ed12525f75b53b0e4e13a087da763a56548314da1a717bd | f38c7758a7ebe02bb23c52ac7618d26531e755cc6cc5c23f63b6e2bab274b98f |
  | `wayfinder-location.onnx` | 37,810,602 | b95c9f8564a8506afc87b8865974b1cf2b2e1919e7d584155d97a18fc80089bb | 33403c52f9b3b3d151867a745bf45879faea46fd77cb8792f281a4eac5aef87c |
  | `wayfinder-gallery.bin` | 103,200,020 | 7045889ad1845f9b26058df39424c2b42889c6e1cccdd33aa55c7eb3bfe56b36 | 49d33cec1f7d93fa00ea6de8e40903c7e83efaced57b722c218fcc2f3abbab40 |

- **Used for:** the Wayfinder plugin (`jl.wayfinder`, Round 24): a guess of where a photo without GPS was taken, kept as a suggestion until the person confirms it.
- **Checked:** `photos-ml/tests/wayfinder_model.rs` holds the ONNX image encoder to PyTorch on five CC0 photos (cosine ≥ 0.99 and the same top-1 gallery place, on CPU and DirectML).
- **Measured (Round 24 W2 Step 3, `scripts/ml/eval_wayfinder.py`):** on 500 camera-GPS photos of a library copy with their location hidden, raw GeoCLIP has a median error of 526 km (63 % within 750 km) and runs at 21.6 photos/s on DirectML including decode (FindMyPhoto's CPU PyTorch path: 0.16 photos/s). PLONK_YFCC, the challenger, was slower (0.59 photos/s) and less accurate, so it is **not adopted**. Numbers in the plan's W2 *As built*.
- **Why fp16 for the image tower only:** it halves the 1.2 GB fp32 file at a cosine of 0.9998 to PyTorch on random input; the location encoder stays fp32 because its Fourier features at sigma 256 are sensitive to rounding.
- **Not in the repo:** the weights are uploaded by hand as a release asset (never committed).

## clip-vit-b32-image-fp16.onnx — `clip-vit-b32-fp16@1`

- **What:** the image tower of OpenAI CLIP ViT-B/32, exported through OpenCLIP 2.26.1, weights in fp16.
- **Source:** OpenAI CLIP weights (https://github.com/openai/CLIP), checkpoint SHA-256 40d365715913c9da98579312b702a82c18be219cc2a73407c4526f58eba950af.
- **Licence:** MIT (OpenAI CLIP). OpenCLIP (export tooling): MIT.
- **SHA-256:** 0e08a2484ca1a1a5bc12cfd31f6cc6e8744c919bd6a132400d7cbcb8f44deeb1 · 176,214,230 bytes.
- **Used for:** one 512-d embedding per photo (memory curation, slideshow grouping, transitions, music mood).
- **Why fp16:** dynamic int8 moved embeddings to cosine 0.970 of fp32 — too far for thresholds at 0.85 and 0.96.

## Aesthetic head (inside models.json, no file)

- **What:** LAION aesthetic predictor linear head for CLIP ViT-B/32, `sa_0_4_vit_b_32_linear.pth` (512 weights + bias).
- **Source:** https://github.com/LAION-AI/aesthetic-predictor · **Licence:** MIT.

## Prototypes (`src-tauri/crates/photos-ml/prototypes.json`)

- Fixed phrases embedded offline with the same CLIP model's text tower. Derived data; MIT as above.

## beat-this-small.onnx — `beat-this-small@1`

- **What:** the `small0` checkpoint of *Beat This!* (Foscarin, Schlüter, Widmer, ISMIR 2024), beat and downbeat activations (sigmoid applied in the export).
- **Source:** https://github.com/CPJKU/beat_this · checkpoint https://cloud.cp.jku.at/public.php/dav/files/7ik4RrBKTS273gp/small0.ckpt
- **Licence:** MIT. The README: "The code and the published model weights are released under the MIT license. Note that some of the training files are fully copyrighted or under limited Creative Commons licenses, and it is up to the user to assess whether this may impact their use case." Lumen ships the weights only, never training data; the weights' own licence is MIT, which passes ML design §2.8.
- **SHA-256:** 8ce9ac22967ff53551f5f319ff11c2e442fc5c9b372205c13f3ff786b56c998c · 9,274,894 bytes.
- **Verdict (2026-09-22):** shipped. ONNX output matches PyTorch within 2.2e-6 on the reference signal.

## clip-vit-b32-text-fp16.onnx — `clip-vit-b32-text-fp16@1` (Phase 7b)

- **What:** the text tower of the same OpenAI CLIP ViT-B/32, fp16, with end-of-text pooling by a float argmax (ONNX Runtime's CPU ArgMax has no int64 kernel; identical to `encode_text`).
- **Licence:** MIT (OpenAI CLIP). **SHA-256:** b879d9e2ee5364962a5782cc7ddd5c49590ef5689f460835536487c4aac18025 · 127,465,976 bytes.
- **Used for:** natural-language search, and the `tag:*` scene prototypes (embedded offline).

## clip-tokenizer.json — `clip-tokenizer@1` (Phase 7b)

- **What:** CLIP's byte-pair tokenizer, from https://huggingface.co/openai/clip-vit-base-patch32. **Licence:** MIT. **SHA-256:** b556ac8c99757ffb677208af34bc8c6721572114111a6e0aaf5fa69ff0b8d842 · 2,224,041 bytes.

## face-yunet-2026may.onnx — `yunet-2026may@1` (Phase 7b)

- **What:** YuNet face detector (dynamic input), https://github.com/opencv/opencv_zoo/tree/main/models/face_detection_yunet. **Licence:** MIT (Shiqi Yu). **SHA-256:** ebafce4e3c118d6554634be5c27ab333b4c047a9a8c3faf1d7cf93101c22f0f0 · 229,738 bytes.

## face-sface-2021dec.onnx — `sface-2021dec@1` (Phase 7b)

- **What:** SFace face embedding (MobileFaceNet, 112×112 aligned → 128-d), https://github.com/opencv/opencv_zoo/tree/main/models/face_recognition_sface. **Licence:** Apache 2.0. **SHA-256:** 0ba9fbfa01b5270c96627c4ef784da859931e02f04419c829e83484087c34e79 · 38,696,353 bytes.
- **Excluded:** InsightFace's SCRFD and ArcFace weights — non-commercial research licences.
- **Open question (2026-10-03):** the weights are published under Apache 2.0, but which data set trained them is undocumented. The SFace paper's repository names CASIA-WebFace, VGGFace2 and MS1MV2, all research-only. An upstream issue asks exactly this for this file's hash, with no answer so far (https://github.com/opencv/opencv_zoo/issues/313). Lumen relies on the published licence. Watch the issue before a commercial release.
- **Vetted as replacements in Round 4I (none qualifies):** EdgeFace: code BSD-3, weights CC BY-NC-SA 4.0, trained on WebFace260M (https://huggingface.co/Idiap/EdgeFace-Base). FaceNet-PyTorch: code MIT, weights trained on VGGFace2 / CASIA-WebFace (research-only). AdaFace and other ArcFace-style models: weights trained on MS1MV2 / WebFace4M (research-only). Models trained on DigiFace-1M: the data set's R-UDA forbids commercial use (https://github.com/microsoft/DigiFace1M).

## denoise-nafnet-sidd-w32-fp16.onnx — `nafnet-sidd-w32@1` (Phase 8d)

- **What:** NAFNet (Chen, Chu, Zhang, Sun, ECCV 2022), width 32, trained on SIDD for real smartphone noise; sRGB 0–1 in and out, run in 512 px tiles overlapping by 32 (sizes a multiple of 16). Exported by `scripts/ml/export_8d.py` (opset 17, dynamic H/W, fp16 weights with fp32 input and output).
- **Source:** architecture https://github.com/megvii-research/NAFNet at `2b4af71ebe098a92a75910c233a3965a3e93ede4`; checkpoint `NAFNet-SIDD-width32.pth` (the repository links it on Google Drive; fetched from the mirror https://huggingface.co/nyanko7/nafnet-models, SHA-256 89c70e808d1783b6c07911306e106aaf0d4f7f3da8c61078b99ff7f8929a26f4).
- **Licence:** code MIT ("Copyright (c) 2022 megvii-model") with BasicSR's Apache 2.0 parts; the weights are published by the repository with no other terms. Training data SIDD: "The dataset and the associated code repositories are under the MIT License" (https://abdokamel.github.io/sidd/). Passes ML design §2.8.
- **SHA-256:** 9b02d22a4f06a3ee57302d09860a6443862cd5096803a21dfdf5772a20616885 · 59,281,558 bytes.
- **Measured (2026-09-26, this machine: RTX 3080 Ti, 4 CPU threads).** Test: three 512 px crops (the Apollo 11 fixture and two library copies) with synthetic noise (Poisson at ~60 photons per unit, about a phone at ISO 3200, plus Gaussian σ = 10/255).
  - Quality: PSNR +10.9 dB on average (20.3 → 26.8, 20.2 → 30.7, 19.5 → 35.1). fp16 ONNX within 0.0005 of PyTorch on these images.
  - Speed per 512 tile: DirectML fp16 60 ms; CPU fp16 3.4 s; CPU fp32 2.4 s.
  - Measured through the Rust runner on a 24 MP copy (117 tiles): **9.1 s on DirectML** and **4.6 min on the CPU**, beyond the plan's ~60 s. The CPU run is a background job with progress and Cancel.
- **Verdict:** shipped. SCUNet (below) denoised the synthetic noise better, but it was trained on that kind of synthetic noise; NAFNet was trained on real phone noise, runs on DirectML, and is 3.6× faster on the CPU.

## ocr_detection.onnx / ocr_recognition.onnx — `pp-ocrv5-mobile-det@1` / `pp-ocrv5-latin-mobile-rec@1` (Round 13) — vetted 2026-10-31

- **What:** PaddleOCR's PP-OCRv5 mobile text detector (DB, any script) and its Latin recogniser (CTC, `ppocr_keys` Latin dictionary: ASCII plus the accented Latin letters, č ć š ž đ included). Converted with `paddlex --paddle2onnx` by `scripts/ml/export_13.py`; pre/post-processing ported to Rust in `photos-ml::ocr`. Dev-folder roles `ocr_detection` / `ocr_recognition`.
- **Source:** https://huggingface.co/PaddlePaddle/PP-OCRv5_mobile_det (revision `0d63e78e2b680928f6b1747d76a08db6e645efb7`), https://huggingface.co/PaddlePaddle/latin_PP-OCRv5_mobile_rec (revision `ab2cd5cc5fa6309be2e5acdfe66eca2c2c127d57`); code https://github.com/PaddlePaddle/PaddleOCR.
- **Licence:** Apache-2.0 for the code (repository `LICENSE`) and both model cards (`license: apache-2.0`), checked 2026-10-31. Commercial use: yes.
- **SHA-256 / bytes:** recorded in the dev entries the export script writes; copied here when the models ship.

## Pets (Round 4C) — vetted 2026-10-03, shipped 2026-10-04

The rule for permissively licensed weights (ML design §2.8): the licence of the code **and** of the weights must allow commercial use. Ultralytics YOLO (AGPL-3.0) and anything with non-commercial weights are out. All candidates below were checked at their sources on 2026-10-03.

**Detectors (COCO `cat` = 15, `dog` = 16):**

| Candidate | Code | Weights | Size / COCO AP | Notes |
|---|---|---|---|---|
| **D-FINE-S, COCO-only** (`dfine_s_coco.pth`, https://github.com/Peterande/D-FINE) | Apache-2.0 | GitHub Releases of `Peterande/storage`, under the repo's Apache-2.0 | 10 M params, 48.5 AP | The README warns that "Checkpoints trained or pretrained on Objects365 (`*_obj365`, `*_obj2coco`) may be subject to the Objects365 dataset terms and should not be assumed to be commercially cleared under the D-FINE license" — so **only the COCO-only checkpoints** qualify. ONNX export is in the repo (`tools/deployment/export_onnx.py`, with its post-processor: labels, boxes, scores). |
| RT-DETR-R18 / RT-DETRv2-S, COCO-only (https://github.com/lyuwenyu/RT-DETR) | Apache-2.0 | GitHub releases under the repo's licence | 20 M params, 46.5 / 48.1 AP | The same Objects365 caveat applies to its `*_obj365` variants (49.2 AP). Twice D-FINE-S's size for slightly lower AP. |
| YOLOX-s (https://github.com/Megvii-BaseDetection/YOLOX) | Apache-2.0 | GitHub Releases under the repo's licence | 9 M params, 40.5 AP | COCO only, the simplest post-processing (grid decode + NMS), long proven in ONNX Runtime. Weaker on small animals. |

**Identity embedders (who this animal is):**

| Candidate | Code | Weights | Size | Notes |
|---|---|---|---|---|
| **DINOv2 ViT-S/14** (https://github.com/facebookresearch/dinov2) | Apache-2.0 | Apache-2.0 ("DINOv2 code and model weights are released under the Apache License 2.0"); only Cell-DINO and XRay-DINO are non-commercial, and are not candidates | 21 M params, 384-d | Self-supervised features are known to tell individual animals apart better than CLIP's semantic ones; one more ~45 MB fp16 download. |
| CLIP ViT-B/32 image tower (already shipped) | MIT | MIT | 0 extra bytes | Tuned for *what* is in a picture, not *which* one; the measurement in Task 1 decides whether it is enough. |

**Training data**, as for the models above: COCO's images are Flickr photos under their own Creative Commons licences (annotations CC BY 4.0); DINOv2 was trained on LVD-142M, curated from public web images. Lumen ships weights only, never training data, and each weight file's own licence is what §2.8 judges.

**Verdict (measured on public data only, `scripts/ml/eval_pets.py`; Julian chose at the gate):** shipped **D-FINE-S, COCO-only** (`dfine-s-coco@1`, `pet-dfine-s-coco.onnx`, 41.8 MB, fp32 because in fp16 it finds nothing) and **DINOv2 ViT-S/14** (`dinov2-vits14-fp16@1`, `pet-dinov2-vits14-fp16.onnx`, 44.3 MB). D-FINE on COCO val2017 at 0.5, one reading per animal: cats 90 %, dogs 87 %, precision 0.90. DINOv2 on DogFaceNet (500 dogs, 2,323 detector crops; CC BY 4.0, evaluation only, not shipped): AUC 0.977 against the CLIP tower's 0.874, so CLIP alone was not enough. The shipped CLIP tower still checks each crop (drawing / toy) and settles cat or dog when ≥ 0.95 sure (`species:*` prototypes). Both files go on the `models-v1` release beside the others. Regression fixtures: three public-domain White House photos of Socks the Cat and Buddy the Dog (US federal works, Wikimedia Commons / DPLA).

**Evaluation data** (used to measure, never shipped, never trained on): Cat Individual Images (518 cats, CC BY 4.0), DogFaceNet (1,393 dogs, CC BY 4.0), Dogs-World (126,550 dogs, CC0), COCO val2017 cats, dogs and other animals (annotations CC BY 4.0), Oxford-IIIT Pet (CC BY-SA 4.0). PetFace is not used (non-commercial, research institutions only).

**Test fixtures in the repo** (never in the app bundle): `photos-ml/tests/fixtures/socks-buddy-*.jpg`, three White House photographs of Socks the Cat and Buddy the Dog: US federal works, public domain (Wikimedia Commons, via DPLA).

## Excluded

- **Ultralytics YOLO** (v5/v8/v11 and their weights): AGPL-3.0 — fails §2.8.
- **D-FINE / RT-DETR Objects365 checkpoints** (`*_obj365`, `*_obj2coco`): may be bound by the Objects365 dataset's terms; not assumed commercially cleared.
- **`AvitoTech/DINO-v2-small-for-animal-identification`** (and its SigLIP2/CLIP siblings): weights Apache-2.0, but fine-tuned on **PetFace**, whose repository states "Our PetFace dataset, code in this repository, and pretrained models are for non-commercial research purpose only". Excluded for a commercial release.
- **PetFace** itself: non-commercial research only; not used even for evaluation.

## Open question: training data

- **YuNet** (`yunet-2026may@1`): weights MIT, trained on WIDER FACE, which is **CC BY-NC-ND**. **SFace**: weights Apache-2.0, training data unclear (opencv_zoo#313). Whether a model inherits its training data's non-commercial terms is unsettled; moot for a non-commercial project, but a fork that sells Lumen should look again.

- **madmom** beat/downbeat weights: CC BY-NC-SA (non-commercial) — fails §2.8.
- **SCUNet** `scunet_color_real_psnr` (https://github.com/cszn/SCUNet, Apache 2.0; weights in KAIR's MIT releases; trained on synthetic noise over DIV2K and others, whose terms are research-oriented): licence-acceptable like Beat This!, but **segfaults under DirectML** (ONNX Runtime 1.24.4) and takes 14.7 s per 512 tile on the CPU (fp16; 8.6 s fp32), 30+ minutes for 24 MP. PSNR +18.5 dB on the synthetic test above.

## Retouch (Round 11B) — training data, vetted 2026-10-06

Lumen's own model (`retouch-v1`, trained from scratch, no third-party weights). Only the trained weights ship, never training images. Shipped weights train only on sources marked **ship** below (`scripts/ml/lumen_train/datasets/registry.toml`, enforced by `train.py --ship`).

| Source | Licence / terms (checked 2026-10-06) | Verdict |
|---|---|---|
| **Wikimedia Commons**, per file CC0 / Public Domain / CC BY (https://commons.wikimedia.org, API with a User-Agent per the bot policy) | Per-file licences; CC0 and PD have no conditions; CC BY needs attribution | **ship** (CC0, PD, CC BY with a credits list in `credits.csv` beside the dataset). CC BY-SA and any NC: **excluded** |
| **Openverse** (https://openverse.org, API) filtered to CC0 / PDM | The aggregator's filter; each item keeps its source licence | **ship** (CC0 / PDM only, licence re-checked per item) |
| **Flickr Commons** (*no known copyright restrictions*) | Institution-declared, per photo | **ship** (as above, per item) |
| **raw.pixls.us** RAW samples | Mostly CC0; some files carry other licences ("do check first") | **ship** for the CC0 files only (RAWs, no edits: synthetic pairs) |
| **pixls.us Play Raw** RAWs | "raw files must be freely licensed", licence per post (CC BY-SA, CC BY-NC-SA, CC0…) | the **CC0** RAWs only, synthetic pairs |
| **pixls.us Play Raw** edits and sidecars | Posts and attachments are **CC BY-NC-SA 4.0** (discuss.pixls.us ToS) | **excluded** (non-commercial) |
| **Unsplash** | Terms §8 prohibit using the Images "in connection with any machine learning and/or artificial intelligence datasets (e.g., training any machine learning and/or artificial intelligence models)" unless licensed via https://unsplash.com/data | **excluded** (a paid data licence could change this) |
| **Pexels** | ToS: scraping/automated collection "strictly prohibited … including without limitation for machine learning purposes"; bulk copying needs permission | **excluded** |
| **MIT-Adobe FiveK**, **PPR10K** | Research / non-commercial use | **excluded** (not even for evaluation) |
| Julian's own catalog / exports | His own photos and edits | **evaluation and his on-device personal layer only**, never shipped weights (`commercial-freemium-release`) |
| **Julian's RAW + edit pairs** (`julian-pairs`, `C:/lumen-train/eval-library`: `NEF-originals/` + `Edited/`) | His own photos and edits; his decision, 2026-10-07, to train shipped weights on them (the one exception to the row above) | **ship** (`commercial = true`, `--val 12 --test 18`; only the test split is on the eval sheet, never trained on) |
| Photographers who grant Lumen a written licence to their RAW + Lightroom edits | Per agreement | **ship** once a signed licence is on file (registry `licence = "agreement:<file>"`) |
| **Photographer (contributor)**: edited JPEGs (`contributor-finals`) and RAWs with their edited JPEGs (`contributor-pairs`) | His permission for training Lumen's shipped models, given 2026-10-07; Julian keeps the written note; no attribution required unless he asks | **ship** (`commercial = true`, splits `--val 12 --test 18`; his test photos are never trained on) |

**Lumen Prism (Round 14)** is trained on `julian-pairs` to edit like Julian does (warm-started from a Wikimedia-only run). It is **for Julian's use**: installed in his dev folder only, **never shipped without his say**; its `model.json` licence reads `trained on julian-pairs (Julian's own photos and edits), wikimedia-cc0-pd (…), wikimedia-cc-by (…)`.

**Consequence:** no openly licensed *before/after pairs with a photographer's edit* allow commercial training today. Shipped v1 learns from **synthetic pairs** made from commercially usable finals and RAWs. The RAW + Lightroom pair builder is built and used for evaluation (Julian's catalog) and for any licensed photographers' sets later.

## Candidates for 8f (not shipped)

Shortlist for Develop's AI masks (**subject**, **sky**, **person / skin**), researched 2026-11-01 (Julian OK'd the research; 8f itself has not started). Nothing is downloaded or built. The rule is the same as for pets (ML design §2.8): the licence of the code **and** of the weights must allow commercial use, and non-commercial training data goes under *Open question* like YuNet. Every licence below was read at its source on 2026-11-01. Develop's preview is 2048 px on the long edge (`develop_preview::PREVIEW_EDGE`): each model runs at its own input size and its mask is upsampled to the render, where Rust rasterises the `subject` / `sky` shape (Round 4 roadmap, 8f). **Speeds are vendor numbers or estimates, not measured here**; 8f's first task measures them on this machine (RTX 3080 Ti / DirectML, 4 CPU threads) as 8d and 4C did.

| Candidate | Covers | Code | Weights | Training data | ONNX | Size | Speed (published → expected here) |
|---|---|---|---|---|---|---|---|
| **EoMT-S, COCO panoptic** (`tue-mps/coco_panoptic_eomt_small_640_2x`; https://github.com/tue-mps/eomt) | sky (`sky-other-merged`), person, the 80 COCO things (the subject when it is one of them) | MIT ("Copyright (c) 2025 Mobile Perception Systems Lab at TU/e", https://github.com/tue-mps/eomt/blob/master/LICENSE) | MIT (Hugging Face card metadata, https://huggingface.co/tue-mps/coco_panoptic_eomt_small_640_2x) | COCO 2017 panoptic: annotations CC BY 4.0, images Flickr under their own licences ("The COCO Consortium does not own the copyright of the images", https://cocodataset.org/#termsofuse), the same footing as the shipped D-FINE-S; backbone DINOv2 ViT-S (Apache-2.0, vetted above for pets) | not published; a plain ViT with a mask head, exported with `torch.onnx` (to check in 8f) | 24 M params (~96 MB fp32, ~48 MB fp16) | 330 FPS on an H100 at 640² (model zoo) → est. 20–50 ms DirectML, 0.2–0.5 s CPU |
| EoMT-B, COCO panoptic (`tue-mps/coco_panoptic_eomt_base_640_2x`) | as EoMT-S, better masks (PQ 51.6 vs 46.7) | MIT | MIT (card metadata) | as EoMT-S (DINOv2 ViT-B) | as EoMT-S | 93.7 M params (~190 MB fp16) | 261 FPS on an H100 → est. 3–4× EoMT-S |
| **MediaPipe Selfie Multiclass** (256²; https://developers.google.com/edge/mediapipe/solutions/vision/image_segmenter) | a person split into hair, **body-skin**, **face-skin**, clothes, accessories, background | Apache-2.0 (MediaPipe) | Apache-2.0 ("LICENSED UNDER Apache License, Version 2.0", model card https://storage.googleapis.com/mediapipe-assets/Model%20Card%20Multiclass%20Segmentation.pdf) | not published ("trained on thousands of samples with diverse identities"); the card reports mean IoU 77.2 (256 model) / 81.1 (512 model, not offered on the page) and a fairness check across skin tones (worst group 68.3 / 71.9, the darkest tones) | `.tflite` only; needs tf2onnx / tflite2onnx (a ViT with custom ops: check that it converts) | not stated | 218 ms CPU / 71 ms GPU on a Pixel 6 → est. < 50 ms on a desktop CPU |
| MediaPipe Selfie Segmenter (256² / 144×256) | person vs background only | Apache-2.0 | Apache-2.0 (model card https://storage.googleapis.com/mediapipe-assets/Model%20Card%20MediaPipe%20Selfie%20Segmentation.pdf) | not published | `.tflite` | 249 KB | 33 ms on a Pixel 6 |
| SAM 2.1 hiera-tiny (https://github.com/facebookresearch/sam2) | any region from a point or box (refines another model's mask; a click-to-select tool) | Apache-2.0 | Apache-2.0 ("The SAM 2 model checkpoints, SAM 2 demo code … and SAM 2 training code are licensed under Apache 2.0") | SA-V (CC BY 4.0, https://ai.meta.com/datasets/segment-anything-video/) plus, per the paper (arXiv 2408.00714), SA-1B, whose page says "Research purposes only" (https://ai.meta.com/datasets/segment-anything/), and internal licensed video → **Open question**, like YuNet | community exports (encoder + decoder), not official | 38.9 M params | 91 FPS on an A100 (video, README) → est. 0.5–1 s CPU for the 1024² encoder, once per photo |
| BiRefNet_lite (https://huggingface.co/ZhengPeng7/BiRefNet_lite) | **subject** (salient object of any class, fine edges) | MIT (https://github.com/ZhengPeng7/BiRefNet) | MIT (card metadata) | **DIS5K** ("trained on DIS-TR"); DIS5K's terms: "The Dataset is available for non-commercial use in research or educational purpose … commercial use of this dataset is prohibited even after copying, editing, processing" (`DIS5K-Dataset-Terms-of-Use.pdf` in https://github.com/xuebinqin/DIS) → **Open question** | official ONNX for every checkpoint (GitHub / Google Drive) | 44.4 M params | 93.8 ms ONNX on an A100 at 1024² → est. 0.2–0.4 s DirectML, several s CPU |

**Excluded for 8f** (licences read at the source, 2026-11-01):
- **Mask2Former** (all checkpoints): code MIT, but the model zoo says its models "are licensed under the Creative Commons Attribution-NonCommercial 4.0 International License" (https://github.com/facebookresearch/Mask2Former/blob/main/MODEL_ZOO.md).
- **SegFormer** (NVIDIA): "The Work and any derivative works thereof only may be used or intended for use non-commercially" (https://github.com/NVlabs/SegFormer/blob/master/LICENSE). Community ONNX ports (e.g. sky / water / person on ADE20K) inherit that.
- **BRIA RMBG-2.0**: CC BY-NC 4.0; "Commercial use is subject to a commercial agreement with BRIA" (https://huggingface.co/briaai/RMBG-2.0).
- **Anything whose sky labels come from ADE20K only**: ADE20K's images are for "non-commercial research and educational purposes" (https://ade20k.csail.mit.edu/terms/; its annotations are BSD-3). OneFormer (code MIT) states no licence for its checkpoints and trains on ADE20K / COCO / Cityscapes: not a candidate until its weights' terms are stated.
- **Sky-Segmentation-and-Post-processing** (U²-Net sky, https://github.com/xiongzhu666/Sky-Segmentation-and-Post-processing): code MIT, but no licence for the weights and no training data named; the good model was never published ("used in our product"), only a 2 MB u2netp demo. Community re-uploads labelled MIT don't change that.
- **IS-Net / DIS, InSPyReNet, BEN2 base**: code Apache-2.0 / MIT / MIT (BEN2's weights MIT), but all trained on DIS5K (non-commercial, above); IS-Net and InSPyReNet state no licence for their weights; BEN2 adds a proprietary 22 K set. Same open question as BiRefNet and no better than it.
- **U²-Net** (`u2net.pth`, `u2netp.pth`): code Apache-2.0, weights with no stated licence, trained on DUTS-TR (no licence found).

**Recommended pick per mask** (to confirm with 8f's measurement on public photos, never Julian's):
- **Sky → EoMT-S, COCO panoptic.** The only candidate with clean code, weights and data that knows *sky*; its backbone is DINOv2, already shipped for pets. Its 640² mask is coarse at sky / tree edges, so Rust refines it at render resolution (e.g. a guided filter on the preview's luminance) instead of using it raw. EoMT-B if S's edges disappoint and the time allows.
- **Person / skin → MediaPipe Selfie Multiclass** for skin (face + body), hair and clothes, with **EoMT-S's person** masks to find people beyond selfie framing (crop each person's box, run Multiclass on it). Both permissive; Multiclass has a published skin-tone fairness check, which a skin tool needs.
- **Subject → EoMT-S's most prominent thing** (largest, most central, sharpest instance; people and pets first) for 8f v1. A class-agnostic salient model (BiRefNet_lite is the best found) would do better on still lifes and objects outside COCO, but every one found trains on DIS5K, so it waits on Julian's answer to the training-data question (*Open question* above). SAM 2.1-tiny (point refine) has the same question through SA-1B.

So one model (EoMT-S, ~48 MB fp16) covers sky, person and subject, and MediaPipe Multiclass adds skin.

### Lumen Facet: chosen and measured (2026-11-02; uploaded to `models-v1` 2026-11-05)

Julian's answers: Sky, Subject, Background and Person first, so **MediaPipe Multiclass is not converted** for now (its `.tflite` conversion is still unproven); the models trained on non-commercial data stay out.

- **Licence re-read at the source (2026-11-02):** code MIT ("Copyright (c) 2025 Mobile Perception Systems Lab at TU/e", `github.com/tue-mps/eomt/blob/master/LICENSE`); weights `license: mit` in the Hugging Face card data, revision `10f5326794a8ce7e0e975791033ae1c13aec3767`. Training data as in the table above.
- **Export:** works with `scripts/ml/export_8f.py` (`transformers` 4.57.1, SciPy needed, torch 2.6.0 CPU, opset 17). Against PyTorch on a public photo: fp32 max diff 0.0000 (class) / 0.0010 (mask logits); fp16 0.0197 / 0.5430. Outputs 200 queries, 133 labels + no-object, masks 160×160; `sky-other-merged` is label 119, `person` 0, stuff starts at 80.
- **File:** `facet-eomt-s-coco-fp16.onnx`, in the dev folder (`build/models`, git-ignored) and, since 2026-11-05 on Julian's go, on `05Narwhal/lumen-models` release `models-v1` (`gh release upload`; the asset reports 48,127,830 bytes). SHA-256 `d86729c15f0ae71b6073241d751a0644e50c7b50b662189f4d7ce2060c066754`, 48,127,830 bytes. `facet-eomt-s-coco-labels.json` is not needed at run time (nothing in the code reads it) and was not uploaded.
- **Download check (2026-11-05, `tauri dev` on a temp library, temp `LUMEN_APP_DATA_DIR`, four public photos, driven over CDP):** turning *AI masks (Facet)* on fetched the file into the temp models folder with the same SHA-256; `ai_masks_detect` found Subject and Background on the two photos tried (1.9 s for the first, with model load; 120 ms for the second); Develop's *New mask* menu shows the AI section (*No sky found*, Subject, Background, *No people found*, *Detect again*). Not run: a failing download in the app (the reason on the row is unit-tested).
- **fp16 vs fp32 on 25 public photos** (143 segments): IoU median 0.999, p10 0.991; 2 segments under 0.9; segment counts differ on 3 photos. fp16 chosen for its size and its speed on DirectML.
- **Speed, one photo, 3 warm-ups + 7 timed runs. Provisional: the machine is busy (three sessions, GPU at about 30 %), re-run when quiet.**

| Build | Size | DirectML median | CPU 4 threads median | Session load |
|---|---|---|---|---|
| fp16 | 48.1 MB | **36 ms** (36–40) | 2,125 ms (1,830–2,501) | 1.14 s DirectML, 0.23 s CPU |
| fp32 | 96.0 MB | 58 ms (54–60) | 946 ms (934–1,017) | 1.50 s DirectML, 0.38 s CPU |

- **Quality, by eye on 25 public photos from `C:/lumen-train/originals/wikimedia-cc0-pd`** (`scripts/ml/eval_facet.py`; Julian's look still to come): sky holds at trees, domes and a statue's curls. Subject is good on cars, a train, a motorcycle, an eagle, sheep, people. Weak spots: **no subject** for statues, buildings and a rocket (not COCO things, as expected); sky fired on two portraits' plain backdrops and a dark workshop ceiling in 120 more public photos, so sky now needs a score of at least 0.95 (figures below) and a backdrop belongs to the new Background mask, Subject's inverse; one of four jets (measured before Subject's same-kind rule went from a half to a quarter) chosen in a formation; a truck picked over small people in a street scene; people in a dark fire scene partly missed.
- **Stored size:** a mask's weights deflate to about 1.9 KB at the median, 16.9 KB at most, about 15 KB per photo.
- **Padding (found in P1, 2026-11-02):** `EomtImageProcessor` pads with black before normalising; the first `eval_facet.py` padded with mean grey after it. Inside the photo the two agree within 1–2 levels, but the padding moves the model's scores, by up to about 0.01 on four test photos and by 0.06 on one backdrop. Fixed in the script and in Rust. The sky scores here are from after the fix; the fp16 vs fp32 comparison, the speed table and the stored sizes above are from before it and were not re-measured.
- **Sky's score bar, 0.95:** in the same 120 public photos, false skies scored 0.93 and 0.92 (portrait backdrops) and 0.78 (a dark workshop ceiling). Real skies scored 0.98 and 0.99 in photos with people, and at least 0.99 on 49 of the 52 person-free photos where sky fired; the other three were 0.92 (an old battle drawing), 0.87 (the Moon against black space) and 0.62 (2 % of a balloon view). Three false skies is a thin sample, so the bar may need tuning; P4's real check includes more portraits with real sky behind them.
- **More sky evidence (P3b, 2026-11-03, `scripts/ml/sky_facet.py`):** all 3,180 kept public photos through EoMT-S fp16; 50 new photos (outside the 120 above) where a person fills at least 4 % of the frame and sky fires at any score, each called real or not by eye. **38 real skies:** 37 at 0.972 or more, one misty night sky (Christ the Redeemer in fog) at 0.863, dropped by the bar. **11 not real** (and one unclear backdrop at 0.956): dropped by the bar, black space at 0.865, a stage at 0.833, a painted mural at 0.804, a car show's hall at 0.755; **kept by the bar:** two ballet stage backdrops (0.994, 0.995), a blue studio backdrop behind a speaker (0.992), an engraving's drawn sky (0.989), black space behind an astronaut (0.984), a painted backdrop behind a statue (0.976) and a wall of fire (0.958). False and real skies overlap from 0.97 up, so no score separates them: a bar of 0.99 would drop four real skies and four false ones and still keep three. **The bar stays 0.95.** It removes the grey portrait backdrops it was set for; stage and blue studio backdrops pass, and the tint shows the person what Sky covers before they use it.

### EoMT-B measured, not adopted (P3b, 2026-11-03)

- **Model:** `tue-mps/coco_panoptic_eomt_base_640_2x`, revision `1ac497947a2410fdb05279c5907c5209af37b9f1`, `license: mit` in the card data (re-read 2026-11-03), same code and data as S. `scripts/ml/export_8f.py --base`: fp32 374.8 MB (max diff 0.0001 class / 0.0022 mask logits vs PyTorch), fp16 187.5 MB (0.0206 / 0.4662; SHA-256 `627c787faf0c96fbd68363b581fe51766be0175c9bdb38d93a70c87b7ffda92c`, kept in a scratchpad, not in the dev folder).
- **Speed, one photo, 3 warm-ups + 7 timed runs, CPU at about 30 % from other work (provisional):** S fp16 30 ms DirectML / 1,400 ms CPU 4 threads; S fp32 43 / 720 ms; **B fp16 53 / 3,395 ms**; B fp32 91 / 2,050 ms.
- **Quality, S fp16 vs B fp16 on 120 public photos** (`scripts/ml/compare_facet.py`, 24 per title group), post-processed as Rust does: sky IoU median 0.992 (49 photos, none under 0.7; sky fires on two photos only with B), Background 0.986, Subject 0.944 but under 0.7 on 17 of 73; B finds a subject on 11 photos where S finds none, S on 4 where B finds none; B finds 178 people to S's 129. By eye on the 39 photos where they disagree: **B is better on people-shaped things** (a group of children, firefighters in a dark fire scene, three statues taken as people), **S is better on objects** (two engines, a gravy boat, a van seat; B picks people in the background or nothing), most of the rest are equally weak (buildings, ceilings, chocolates), and B adds a false sky at 0.977 on the fire scene.
- **Call: keep S.** B is not clearly better on subjects, its fp16 file takes 3.4 s a photo on CPU (over the ~3 s bar; fp32 is 2.0 s but 375 MB), and it is a 4× download. Revisit with a class-agnostic subject model if the training-data question changes (*Open question* above), not with B.
