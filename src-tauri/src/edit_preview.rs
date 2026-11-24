//! Live edit preview. The first render of an asset decodes its 2560 preview once and keeps
//! a 1024 px copy; every slider change after that is `apply` on 1024 px plus a JPEG encode.
//! Rendered bytes are held by token and served by `photos://edit-preview/{token}`, because
//! bytes never travel over `invoke`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use image::DynamicImage;

// Develop renders often (sliders, preset hovers, 100 % regions): a token must outlive the
// few renders that follow it until the webview has fetched it.
const KEEP_RENDERS: usize = 8;
const KEEP_BASES: usize = 2;
const KEEP_PINNED: usize = 2;

#[derive(Default)]
pub struct PreviewCache {
    next: AtomicU64,
    renders: Mutex<VecDeque<(u64, Arc<Vec<u8>>)>>,
    pinned: Mutex<VecDeque<(u64, Arc<Vec<u8>>)>>,
    bases: Mutex<VecDeque<(i64, Arc<DynamicImage>)>>,
}

impl PreviewCache {
    pub fn put(&self, jpeg: Vec<u8>) -> u64 {
        let token = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let mut r = self.renders.lock().expect("edit previews lock");
        r.push_back((token, Arc::new(jpeg)));
        while r.len() > KEEP_RENDERS {
            r.pop_front();
        }
        token
    }

    /// A render the view keeps showing while sliders move (the before of split / side by side):
    /// it sits outside the ring, so later renders never evict it. Only the latest few are kept.
    pub fn put_pinned(&self, jpeg: Vec<u8>) -> u64 {
        let token = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let mut p = self.pinned.lock().expect("edit previews pinned lock");
        p.push_back((token, Arc::new(jpeg)));
        while p.len() > KEEP_PINNED {
            p.pop_front();
        }
        token
    }

    pub fn get(&self, token: u64) -> Option<Arc<Vec<u8>>> {
        let find = |q: &VecDeque<(u64, Arc<Vec<u8>>)>| q.iter().find(|(t, _)| *t == token).map(|(_, b)| b.clone());
        find(&self.renders.lock().expect("edit previews lock")).or_else(|| find(&self.pinned.lock().expect("edit previews pinned lock")))
    }

    /// The 1024 px unedited image for `asset_id`, loaded once while it is being edited.
    pub fn base<E>(&self, asset_id: i64, load: impl FnOnce() -> Result<DynamicImage, E>) -> Result<Arc<DynamicImage>, E> {
        if let Some((_, img)) = self.bases.lock().expect("edit bases lock").iter().find(|(id, _)| *id == asset_id) {
            return Ok(img.clone());
        }
        let img = Arc::new(load()?);
        let mut b = self.bases.lock().expect("edit bases lock");
        b.push_back((asset_id, img.clone()));
        while b.len() > KEEP_BASES {
            b.pop_front();
        }
        Ok(img)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_latest_renders_and_drops_old_tokens() {
        let cache = PreviewCache::default();
        let tokens: Vec<u64> = (0..10).map(|i| cache.put(vec![i as u8])).collect();
        assert!(cache.get(tokens[0]).is_none(), "only the most recent few are kept");
        assert_eq!(cache.get(tokens[9]).as_deref().map(|v| v.as_slice()), Some(&[9u8][..]));
    }

    #[test]
    fn a_pinned_before_outlives_the_renders_that_follow() {
        let cache = PreviewCache::default();
        let before = cache.put_pinned(vec![42]);
        for i in 0..(KEEP_RENDERS * 3) {
            cache.put(vec![i as u8]);
        }
        assert_eq!(cache.get(before).as_deref().map(|v| v.as_slice()), Some(&[42u8][..]));
        // Only the latest few pinned ones are kept.
        let later: Vec<u64> = (0..KEEP_PINNED + 1).map(|i| cache.put_pinned(vec![i as u8])).collect();
        assert!(cache.get(before).is_none());
        assert!(cache.get(*later.last().unwrap()).is_some());
    }

    #[test]
    fn base_images_are_cached_per_asset() {
        let cache = PreviewCache::default();
        let calls = std::cell::Cell::new(0);
        let load = || {
            calls.set(calls.get() + 1);
            Ok::<_, ()>(image::DynamicImage::new_rgb8(4, 4))
        };
        cache.base(1, load).unwrap();
        cache.base(1, load).unwrap();
        assert_eq!(calls.get(), 1);
    }
}
