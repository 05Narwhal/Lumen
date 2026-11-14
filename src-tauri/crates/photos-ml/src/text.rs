//! Search by meaning (Phase 7b): a sentence to the same space as the photographs' CLIP
//! embeddings. OpenAI's own byte-pair tokenizer (`tokenizer.json`, through the `tokenizers`
//! crate), padded as open_clip pads: start, tokens, end, zeros to 77.

use std::path::Path;

use ort::{inputs, session::Session, value::Tensor};
use photos_core::embedding::{mean, quantize, search_prompts, Vector};
use tokenizers::Tokenizer;

use crate::manifest::ModelEntry;
use crate::MlError;

pub const CONTEXT: usize = 77;
const END_OF_TEXT: i64 = 49407;

/// The 77 token ids open_clip would feed for `text`.
pub fn tokens(tokenizer: &Tokenizer, text: &str) -> Result<Vec<i64>, MlError> {
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let enc = tokenizer.encode(clean, true).map_err(|e| MlError::Runtime(e.to_string()))?;
    let mut ids: Vec<i64> = enc.get_ids().iter().map(|i| *i as i64).collect();
    if ids.len() > CONTEXT {
        ids.truncate(CONTEXT);
        ids[CONTEXT - 1] = END_OF_TEXT;
    }
    ids.resize(CONTEXT, 0);
    Ok(ids)
}

pub struct TextEncoder {
    session: Session,
    tokenizer: Tokenizer,
}

impl TextEncoder {
    pub fn open(model: &Path, tokenizer: &Path, _entry: &ModelEntry) -> Result<Self, MlError> {
        Ok(Self {
            session: Session::builder()?.with_intra_threads(2)?.commit_from_file(model)?,
            tokenizer: Tokenizer::from_file(tokenizer).map_err(|e| MlError::Missing(e.to_string()))?,
        })
    }

    /// `text` as a quantised CLIP embedding, comparable with the photographs'.
    pub fn embed(&mut self, text: &str) -> Result<Vector, MlError> {
        let ids = tokens(&self.tokenizer, text)?;
        let t = Tensor::from_array(([1usize, CONTEXT], ids))?;
        let out = self.session.run(inputs!["tokens" => t])?;
        let (_, data) = out["embedding"].try_extract_tensor::<f32>()?;
        Ok(quantize(data))
    }

    /// A search's query as the mean of its `search_prompts`, all asked in one batch so the
    /// three cost about one run. A model exported with a fixed batch of one is asked in turn.
    pub fn embed_ensemble(&mut self, query: &str) -> Result<Vector, MlError> {
        let prompts = search_prompts(query);
        let mut ids = Vec::with_capacity(prompts.len() * CONTEXT);
        for p in &prompts {
            ids.extend(tokens(&self.tokenizer, p)?);
        }
        let t = Tensor::from_array(([prompts.len(), CONTEXT], ids))?;
        let batch: Option<Vec<Vector>> = match self.session.run(inputs!["tokens" => t]) {
            Ok(out) => {
                let (_, data) = out["embedding"].try_extract_tensor::<f32>()?;
                Some(data.chunks_exact(data.len() / prompts.len()).map(quantize).collect())
            }
            Err(_) => None,
        };
        let each = match batch {
            Some(each) => each,
            None => prompts.iter().map(|p| self.embed(p)).collect::<Result<_, _>>()?,
        };
        mean(&each.iter().map(|v| v.as_slice()).collect::<Vec<_>>()).ok_or_else(|| MlError::Runtime("no prompts".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_match_open_clip() {
        let tokenizer = Tokenizer::from_bytes(include_bytes!("../tests/fixtures/clip-tokenizer.json")).unwrap();
        let reference: serde_json::Value = serde_json::from_str(include_str!("../tests/fixtures/text_reference.json")).unwrap();
        for (phrase, want) in reference["phrases"].as_array().unwrap().iter().zip(reference["tokens"].as_array().unwrap()) {
            let want: Vec<i64> = want.as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
            assert_eq!(tokens(&tokenizer, phrase.as_str().unwrap()).unwrap(), want, "{phrase}");
        }
        let long = "word ".repeat(200);
        let t = tokens(&tokenizer, &long).unwrap();
        assert_eq!(t.len(), CONTEXT);
        assert_eq!(t[CONTEXT - 1], END_OF_TEXT, "a long query still ends with end-of-text");
    }
}
