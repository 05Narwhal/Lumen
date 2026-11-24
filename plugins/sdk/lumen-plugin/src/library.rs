//! The library, read-only (needs `library.read`; feature `places`): dates, places, file names
//! and folders of the person's photos, 200 per page. Never their bytes.

#[cfg(all(target_arch = "wasm32", any(feature = "places", feature = "proposals")))]
mod imp {
    use crate::bindings::lumen::plugin::library as l;
    use crate::{Asset, AssetDetails, AssetFilter, AssetPage};

    /// One page; pass the previous page's `next_cursor` for the next.
    pub fn assets(filter: &AssetFilter, cursor: Option<&str>) -> Result<AssetPage, String> {
        l::assets(filter, cursor)
    }

    /// API 1.4: titles, captions, keywords, people, albums, text and the date's source for up to
    /// 200 assets of a page. An asset that is gone or in the trash is left out.
    pub fn details(ids: &[u64]) -> Result<Vec<AssetDetails>, String> {
        l::details(ids)
    }

    /// Calls `f` with every page in turn until the last one or `f` returns `false`.
    pub fn each_page(filter: &AssetFilter, mut f: impl FnMut(Vec<Asset>) -> bool) -> Result<(), String> {
        let mut cursor: Option<String> = None;
        loop {
            let page = l::assets(filter, cursor.as_deref())?;
            if !f(page.items) {
                return Ok(());
            }
            match page.next_cursor {
                Some(c) => cursor = Some(c),
                None => return Ok(()),
            }
        }
    }
}

#[cfg(all(target_arch = "wasm32", any(feature = "places", feature = "proposals")))]
pub use imp::*;
