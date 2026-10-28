use gpui::{App, ClickEvent, ClipboardItem, Window};

use crate::app::DiscordApp;

/// Actions the bottom-left account popout can perform. Discord's other entries (editing
/// profiles, status changes, account switching, clips and orbs) need APIs this client lacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccountAction {
    ViewProfile,
    Settings,
    CopyUserId,
}

/// A click handler bound to the app entity, matching the popout's existing handler shape.
/// Viewing the full profile keeps the card's global context; the other actions close the
/// popout first.
pub(crate) fn account_action(
    app: &DiscordApp,
    action: AccountAction,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let entity = app.entity.clone();
    move |_, window, cx| {
        cx.stop_propagation();
        entity
            .update(cx, |app, cx| match action {
                AccountAction::ViewProfile => app.open_full_profile(cx),
                AccountAction::Settings => {
                    app.close_profile(cx);
                    app.open_settings(window, cx);
                }
                AccountAction::CopyUserId => {
                    app.close_profile(cx);
                    let user_id = app.store.current_user_id;
                    cx.write_to_clipboard(ClipboardItem::new_string(user_id.to_string()));
                }
            })
            .ok();
    }
}
