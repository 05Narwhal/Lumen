//! Retouch (Round 11B): the slider values Lumen's own model predicts, laid out once here so
//! the training scripts (`scripts/ml/lumen_train`), the model and the app agree.

pub mod params;

pub use params::{decode, encode, onto_draft, Group, Param, RetouchRecipe, PARAMS, RETOUCH_LAYOUT};
