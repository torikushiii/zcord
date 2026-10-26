use super::*;

fn ready_with_contact(email: Option<&str>, phone: Option<&str>, mfa_enabled: bool) -> model::Ready {
    let mut payload: serde_json::Value =
        serde_json::from_str(include_str!("../../discord/fixtures/ready.json")).unwrap();
    payload["user"]["email"] = json!(email);
    payload["user"]["phone"] = json!(phone);
    payload["user"]["mfa_enabled"] = json!(mfa_enabled);
    payload["user"]["verified"] = json!(true);
    serde_json::from_value(payload).unwrap()
}

#[test]
fn ready_account_carries_contact_fields() {
    let ready = ready_with_contact(Some("user@example.com"), Some("+15550100"), true);
    assert!(ready.user.verified);
    let store = Store::from_ready(ready);
    let account = store.account().unwrap();
    assert_eq!(account.username, "owner");
    assert_eq!(account.email.as_deref(), Some("user@example.com"));
    assert_eq!(account.phone.as_deref(), Some("+15550100"));
    assert!(account.mfa_enabled);
}

#[test]
fn ready_account_allows_null_contact_fields() {
    let store = Store::from_ready(ready_with_contact(None, None, false));
    let account = store.account().unwrap();
    assert_eq!(account.username, "owner");
    assert!(account.email.is_none());
    assert!(account.phone.is_none());
    assert!(!account.mfa_enabled);
}

#[test]
fn user_update_refreshes_account() {
    let mut store = Store::from_ready(ready_with_contact(None, None, false));
    let update: model::UserUpdate = serde_json::from_value(json!({
        "id": "1",
        "username": "renamed",
        "email": "new@example.com",
        "phone": "+15550101",
        "mfa_enabled": true
    }))
    .unwrap();
    store.apply(Event::UserUpdate(Box::new(update)));
    let account = store.account().unwrap();
    assert_eq!(account.username, "renamed");
    assert_eq!(account.email.as_deref(), Some("new@example.com"));
    assert_eq!(account.phone.as_deref(), Some("+15550101"));
    assert!(account.mfa_enabled);

    let update: model::UserUpdate = serde_json::from_value(json!({
        "id": "1",
        "email": null,
        "phone": null
    }))
    .unwrap();
    store.apply(Event::UserUpdate(Box::new(update)));
    let account = store.account().unwrap();
    assert!(account.email.is_none());
    assert!(account.phone.is_none());
}
