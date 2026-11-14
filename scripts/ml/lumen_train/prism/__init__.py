"""Lumen Prism: Retouch fitted to Julian's own edits (`julian-pairs`), few-shot.

`folds` splits his pairs by photo, `augment` makes many views of each pair, `fit` trains one
model (warm start, partial freeze, 1:3 mix, EMA), `cv` runs the cross-validation, the final
fit and the gate (docs/ML-TRAINING.md, "Lumen Prism")."""
