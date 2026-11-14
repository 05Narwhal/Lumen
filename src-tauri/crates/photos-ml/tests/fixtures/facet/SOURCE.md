# Lumen Facet's fixtures

Written by `scripts/ml/golden_facet.py`, never by hand. The photos are from the Wikimedia Commons CC0 / public-domain set, made smaller.

| File | Commons page | Photo |
|---|---|---|
| `crew.jpg` | pageid 130335077 | File:Artemis 2 Crew Portrait.jpg |
| `valley.jpg` | pageid 122455417 | File:Panoramic view of Jacques-Cartier National Park from the Wolves peak (Les Loups), Quebec, Canada.jpg |
| `car.jpg`, `preprocess.png` | pageid 191655451 | File:1971 Nissan Skyline Hardtop 2000GT.jpg |

- `preprocess.json`: pixels sampled from what `EomtImageProcessor` makes of `preprocess.png`.
- `synthetic.json`: made-up logits and what `eval_facet.py` makes of them.
- `golden.json`: what the model on the CPU and `eval_facet.py` make of the three photos.
