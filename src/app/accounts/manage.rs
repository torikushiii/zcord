use super::modal::{Dialog, MEDIUM, button, dialog};
use super::*;

pub(crate) fn manage_dialog(app: &DiscordApp, cx: &App) -> Option<AnyElement> {
    if !app.accounts.manage_open {
        return None;
    }
    let saved = app.switchable_accounts(cx);
    let full = settings::accounts(cx).len() >= settings::MAX_ACCOUNTS;
    let enabled = !full && app.accounts.busy().is_none();
    let close = app.entity.clone();
    let add = app.entity.clone();
    let body = account_list(
        &saved,
        &app.accounts,
        ListPlace::Manage,
        app.current_account_id(),
        app.entity.clone(),
        cx,
    );
    let footer = button("manage-accounts-add", "Add an account", false, enabled)
        .flex_1()
        .when(enabled, |this| {
            this.on_click(move |_, window, cx| {
                cx.stop_propagation();
                add.update(cx, |app, cx| app.open_add_account(window, cx))
                    .ok();
            })
        })
        .into_any_element();
    Some(dialog(
        Dialog {
            id: "manage-accounts",
            width: MEDIUM,
            title: "Manage Accounts",
            subtitle: Some("Switch accounts, sign in, sign out, go wild."),
            body,
            footer,
        },
        move |_, cx| {
            close
                .update(cx, |app, cx| app.close_manage_accounts(cx))
                .ok();
        },
    ))
}
