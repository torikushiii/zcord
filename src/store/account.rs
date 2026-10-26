use super::*;

#[derive(Clone, PartialEq, Eq)]
pub struct Account {
    pub username: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub mfa_enabled: bool,
}

impl Store {
    pub fn account(&self) -> Option<Account> {
        self.account.clone()
    }
}
