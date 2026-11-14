//! The system's locale, for the date order (Round 11).

use photos_core::settings::{DateOrder, DateOrderSetting};

/// The OS locale as a BCP 47 tag (`en-US`), or empty when the system gives none.
pub fn system() -> String {
    sys_locale::get_locale().unwrap_or_default()
}

/// The order `setting` means on this system.
pub fn date_order(setting: DateOrderSetting) -> DateOrder {
    setting.resolve(&system())
}

/// The order the open settings ask for, resolved.
pub fn current(state: &crate::state::AppState) -> DateOrder {
    date_order(state.settings.read().map(|s| s.search.date_order).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chosen_order_wins_over_the_system() {
        assert_eq!(date_order(DateOrderSetting::Ymd), DateOrder::Ymd);
        assert_eq!(date_order(DateOrderSetting::Auto), photos_core::settings::order_for_locale(&system()));
    }
}
