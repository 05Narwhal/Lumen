"""Round 4 prototypes, added to prototypes.json (keys only; nothing existing changes).

    .venv-ml/Scripts/python scripts/ml/export_round4.py --models build/models

Round 4B: `face:*`, what a detected face's crop is — a real face, or a drawing, a print, a
screen, a meme or a statue — so faces in notebooks, on screens and in memes are not people.
Embedded with the same OpenAI CLIP ViT-B/32 text tower as the 7a/7b prototypes.
"""
import argparse
import json
import pathlib

import open_clip
import torch

REPO = pathlib.Path(__file__).resolve().parents[2]
ML = REPO / "src-tauri" / "crates" / "photos-ml"

# Two phrasings each, averaged, as the scene tags are.
FACE_KINDS = {
    "real": ["a photo of a real person's face", "a close-up photo of a person"],
    "drawing": ["a drawing or cartoon of a face", "an illustration of a face"],
    "print": ["a face printed on paper, in a book or a notebook", "a printed photo of a face on paper"],
    "screen": ["a face on a phone, TV or computer screen", "a photo of a screen showing a face"],
    "meme": ["an internet meme with a face and text", "a meme image with a caption"],
    "statue": ["a statue or a mannequin", "a sculpture of a head"],
}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--models", default="build/models")
    args = ap.parse_args()
    model, _, _ = open_clip.create_model_and_transforms("ViT-B-32", pretrained="openai", cache_dir=args.models)
    model.eval()
    tok = open_clip.get_tokenizer("ViT-B-32")
    path = ML / "prototypes.json"
    protos = json.loads(path.read_text())
    with torch.no_grad():
        for kind, phrases in FACE_KINDS.items():
            t = model.encode_text(tok(phrases))
            t = (t / t.norm(dim=-1, keepdim=True)).mean(dim=0)
            protos["prototypes"][f"face:{kind}"] = [round(x, 6) for x in (t / t.norm()).tolist()]
    path.write_text(json.dumps(protos))
    print("added", [f"face:{k}" for k in FACE_KINDS])


if __name__ == "__main__":
    main()
