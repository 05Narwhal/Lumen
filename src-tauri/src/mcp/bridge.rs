//! Edits to the photo open in Develop, while Develop is on screen, go through the page
//! (Round 16 P2): Rust emits `mcp://request`, `routes/develop/mcp-bridge.ts` applies it
//! through the session as one history step — the user sees the sliders move and their own
//! edits are kept — and answers with `mcp_reply`. No answer in 10 s is `develop_busy`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use photos_core::develop::Panel;
use photos_core::edits::{Develop, Recipe};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::error::{AppError, AppResult};

pub const EVENT: &str = "mcp://request";
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// What to do to a recipe. The page sends it back to `mcp_transform` with the draft on
/// screen, so the change lands on top of the user's own edits; `undo` and `open` are the
/// page's own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, rename = "McpOp")]
pub enum Op {
    Patch {
        #[ts(type = "unknown")]
        patch: Value,
    },
    Mask {
        #[ts(type = "unknown")]
        args: Value,
    },
    Auto { model: String },
    Reset { panels: Vec<Panel> },
    Paste { develop: Box<Develop>, panels: Vec<Panel> },
    Undo,
    Open,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct McpRequest {
    #[ts(type = "number")]
    pub req: u64,
    #[ts(type = "number")]
    pub asset_id: i64,
    /// The history step's label, `AI: …`.
    pub label: String,
    pub op: Op,
}

/// The page's answer: the recipe now on screen, or a refusal with a stable code.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct McpReply {
    pub recipe: Option<Recipe>,
    pub error: Option<String>,
    pub message: Option<String>,
}

pub fn busy() -> AppError {
    AppError {
        code: "develop_busy".into(),
        message: "Develop did not answer in time (the user may be dragging a slider or opening another photo). Try again.".into(),
        detail: None,
    }
}

#[derive(Default)]
pub struct Bridge {
    next: AtomicU64,
    waiting: Mutex<HashMap<u64, mpsc::SyncSender<McpReply>>>,
}

impl Bridge {
    /// Send `op` to the page with `emit` and wait for its answer (no recipe for `open`).
    /// Blocking: tools run on `spawn_blocking`.
    pub fn ask(&self, asset_id: i64, label: &str, op: Op, emit: impl FnOnce(&McpRequest) -> AppResult<()>, timeout: Duration) -> AppResult<Option<Recipe>> {
        let req = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = mpsc::sync_channel(1);
        self.waiting.lock().unwrap_or_else(|p| p.into_inner()).insert(req, tx);
        let sent = emit(&McpRequest { req, asset_id, label: label.into(), op });
        let got = sent.and_then(|()| rx.recv_timeout(timeout).map_err(|_| busy()));
        self.waiting.lock().unwrap_or_else(|p| p.into_inner()).remove(&req);
        let reply = got?;
        match reply.error {
            Some(code) => Err(AppError { code, message: reply.message.unwrap_or_default(), detail: None }),
            None => Ok(reply.recipe),
        }
    }

    /// The page's answer to `req`; one that came too late is dropped.
    pub fn reply(&self, req: u64, reply: McpReply) {
        if let Some(tx) = self.waiting.lock().unwrap_or_else(|p| p.into_inner()).remove(&req) {
            let _ = tx.try_send(reply);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn an_answer_comes_back_and_silence_is_busy() {
        let bridge = Arc::new(Bridge::default());
        let b = bridge.clone();
        let r = bridge.ask(
            7,
            "AI: warmer",
            Op::Undo,
            move |req| {
                assert_eq!((req.asset_id, req.label.as_str()), (7, "AI: warmer"));
                let (b, id) = (b.clone(), req.req);
                std::thread::spawn(move || b.reply(id, McpReply { recipe: Some(Recipe::identity()), error: None, message: None }));
                Ok(())
            },
            TIMEOUT,
        );
        assert_eq!(r.unwrap(), Some(Recipe::identity()));
        let refused = bridge.ask(7, "AI: x", Op::Undo, |_| Ok(()), Duration::from_millis(20)).unwrap_err();
        assert_eq!(refused.code, "develop_busy");
        assert!(bridge.waiting.lock().unwrap().is_empty(), "a late answer finds nobody waiting");
        let b = bridge.clone();
        let err = bridge
            .ask(7, "AI: x", Op::Undo, move |req| {
                b.reply(req.req, McpReply { recipe: None, error: Some("last_step_not_ai".into()), message: Some("no".into()) });
                Ok(())
            }, TIMEOUT)
            .unwrap_err();
        assert_eq!(err.code, "last_step_not_ai");
    }
}
