/// Multi-account management system for trading across multiple protocols
use crate::models::{AccountPurpose, Protocol, TradingAccount};
use crate::utils::{Error, Result};
use std::collections::HashMap;

/// Manages multiple trading accounts
#[derive(Clone, Debug)]
pub struct AccountManager {
    accounts: HashMap<String, TradingAccount>,
}

impl AccountManager {
    /// Create a new account manager
    pub fn new() -> Self {
        Self {
            accounts: HashMap::new(),
        }
    }

    /// Register a new trading account
    pub fn register_account(&mut self, account: TradingAccount) -> Result<String> {
        // Validate account configuration
        account.validate()?;

        // Check for duplicates
        if self.accounts.contains_key(&account.id) {
            return Err(Error::DuplicateAccount);
        }

        let id = account.id.clone();
        self.accounts.insert(id.clone(), account);
        Ok(id)
    }

    /// Get account by ID
    pub fn get_account(&self, id: &str) -> Option<&TradingAccount> {
        self.accounts.get(id)
    }

    /// Get mutable reference to account
    pub fn get_account_mut(&mut self, id: &str) -> Option<&mut TradingAccount> {
        self.accounts.get_mut(id)
    }

    /// Get all accounts
    pub fn get_all_accounts(&self) -> Vec<&TradingAccount> {
        self.accounts.values().collect()
    }

    /// Get accounts by protocol
    pub fn get_accounts_by_protocol(&self, protocol: Protocol) -> Vec<&TradingAccount> {
        self.accounts
            .values()
            .filter(|a| a.protocol == protocol)
            .collect()
    }

    /// Get accounts by purpose
    pub fn get_accounts_by_purpose(&self, purpose: AccountPurpose) -> Vec<&TradingAccount> {
        self.accounts
            .values()
            .filter(|a| a.purpose == purpose)
            .collect()
    }

    /// Get active accounts only
    pub fn get_active_accounts(&self) -> Vec<&TradingAccount> {
        self.accounts.values().filter(|a| a.is_active).collect()
    }

    /// Deactivate account
    pub fn deactivate_account(&mut self, id: &str) -> Result<()> {
        let account = self.get_account_mut(id).ok_or(Error::AccountNotFound)?;
        account.is_active = false;
        account.touch();
        Ok(())
    }

    /// Reactivate account
    pub fn activate_account(&mut self, id: &str) -> Result<()> {
        let account = self.get_account_mut(id).ok_or(Error::AccountNotFound)?;
        account.is_active = true;
        account.touch();
        Ok(())
    }

    /// Update account leverage
    pub fn set_leverage(&mut self, id: &str, leverage: f64) -> Result<()> {
        let account = self.get_account_mut(id).ok_or(Error::AccountNotFound)?;

        if leverage > account.purpose.max_leverage() {
            return Err(Error::InvalidAccountConfig);
        }

        if leverage < 0.0 {
            return Err(Error::InvalidAccountConfig);
        }

        account.current_leverage = leverage;
        account.touch();
        Ok(())
    }

    /// Update account capital allocation
    pub fn set_capital_allocation(&mut self, id: &str, allocation: f64) -> Result<()> {
        let account = self.get_account_mut(id).ok_or(Error::AccountNotFound)?;

        if allocation < 0.0 || allocation > 1.0 {
            return Err(Error::InvalidAccountConfig);
        }

        account.capital_allocation = allocation;
        account.touch();
        Ok(())
    }

    /// Update account max position size
    pub fn set_max_position_size(&mut self, id: &str, max_size: f64) -> Result<()> {
        let account = self.get_account_mut(id).ok_or(Error::AccountNotFound)?;

        if max_size < 0.0 || max_size > 0.5 {
            return Err(Error::InvalidAccountConfig);
        }

        account.max_position_size = max_size;
        account.touch();
        Ok(())
    }

    /// Get total capital allocated
    pub fn total_capital_allocated(&self) -> f64 {
        self.accounts.values().map(|a| a.capital_allocation).sum()
    }

    /// Get total active accounts
    pub fn active_account_count(&self) -> usize {
        self.accounts.values().filter(|a| a.is_active).count()
    }

    /// Get total accounts
    pub fn total_account_count(&self) -> usize {
        self.accounts.len()
    }

    /// Check if account exists
    pub fn account_exists(&self, id: &str) -> bool {
        self.accounts.contains_key(id)
    }

    /// Remove account (for testing/management only)
    pub fn remove_account(&mut self, id: &str) -> Option<TradingAccount> {
        self.accounts.remove(id)
    }

    /// Get account summary
    pub fn get_account_summary(&self, id: &str) -> Result<AccountSummary> {
        let account = self.get_account(id).ok_or(Error::AccountNotFound)?;

        Ok(AccountSummary {
            id: account.id.clone(),
            protocol: account.protocol,
            purpose: account.purpose,
            capital_allocation: account.capital_allocation,
            current_leverage: account.current_leverage,
            is_active: account.is_active,
            max_position_size: account.max_position_size,
        })
    }

    /// List all account summaries
    pub fn get_all_account_summaries(&self) -> Vec<AccountSummary> {
        self.accounts
            .values()
            .map(|a| AccountSummary {
                id: a.id.clone(),
                protocol: a.protocol,
                purpose: a.purpose,
                capital_allocation: a.capital_allocation,
                current_leverage: a.current_leverage,
                is_active: a.is_active,
                max_position_size: a.max_position_size,
            })
            .collect()
    }
}

impl Default for AccountManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary view of a trading account
#[derive(Clone, Debug)]
pub struct AccountSummary {
    pub id: String,
    pub protocol: Protocol,
    pub purpose: AccountPurpose,
    pub capital_allocation: f64,
    pub current_leverage: f64,
    pub is_active: bool,
    pub max_position_size: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_account(id: &str, purpose: AccountPurpose) -> TradingAccount {
        let mut account = TradingAccount::new(
            id.to_string(),
            Protocol::Drift,
            format!("key_{}", id),
            purpose,
        );
        account.capital_allocation = 0.2;
        account
    }

    #[test]
    fn test_register_account() {
        let mut manager = AccountManager::new();
        let account = create_test_account("test-1", AccountPurpose::Scalp);

        let id = manager.register_account(account).unwrap();
        assert_eq!(id, "test-1");
        assert!(manager.get_account(&id).is_some());
    }

    #[test]
    fn test_duplicate_account_error() {
        let mut manager = AccountManager::new();
        let account1 = create_test_account("dup", AccountPurpose::Scalp);
        let account2 = create_test_account("dup", AccountPurpose::Swing);

        manager.register_account(account1).unwrap();
        let result = manager.register_account(account2);

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), Error::DuplicateAccount);
    }

    #[test]
    fn test_get_account() {
        let mut manager = AccountManager::new();
        let account = create_test_account("get-test", AccountPurpose::Scalp);

        manager.register_account(account).unwrap();
        let retrieved = manager.get_account("get-test").unwrap();

        assert_eq!(retrieved.id, "get-test");
        assert_eq!(retrieved.purpose, AccountPurpose::Scalp);
    }

    #[test]
    fn test_get_all_accounts() {
        let mut manager = AccountManager::new();

        let accounts = vec![
            create_test_account("scalp", AccountPurpose::Scalp),
            create_test_account("swing", AccountPurpose::Swing),
            create_test_account("position", AccountPurpose::Position),
        ];

        for account in accounts {
            manager.register_account(account).unwrap();
        }

        assert_eq!(manager.get_all_accounts().len(), 3);
        assert_eq!(manager.total_account_count(), 3);
    }

    #[test]
    fn test_get_accounts_by_protocol() {
        let mut manager = AccountManager::new();

        let mut drift_account = create_test_account("drift-1", AccountPurpose::Scalp);
        drift_account.protocol = Protocol::Drift;

        let mut hyperliquid_account = create_test_account("hpl-1", AccountPurpose::Swing);
        hyperliquid_account.protocol = Protocol::Hyperliquid;

        manager.register_account(drift_account).unwrap();
        manager.register_account(hyperliquid_account).unwrap();

        let drift_accounts = manager.get_accounts_by_protocol(Protocol::Drift);
        assert_eq!(drift_accounts.len(), 1);

        let hyperliquid_accounts = manager.get_accounts_by_protocol(Protocol::Hyperliquid);
        assert_eq!(hyperliquid_accounts.len(), 1);
    }

    #[test]
    fn test_get_accounts_by_purpose() {
        let mut manager = AccountManager::new();

        let scalp1 = create_test_account("s1", AccountPurpose::Scalp);
        let scalp2 = create_test_account("s2", AccountPurpose::Scalp);
        let swing = create_test_account("sw", AccountPurpose::Swing);

        manager.register_account(scalp1).unwrap();
        manager.register_account(scalp2).unwrap();
        manager.register_account(swing).unwrap();

        let scalp_accounts = manager.get_accounts_by_purpose(AccountPurpose::Scalp);
        assert_eq!(scalp_accounts.len(), 2);

        let swing_accounts = manager.get_accounts_by_purpose(AccountPurpose::Swing);
        assert_eq!(swing_accounts.len(), 1);
    }

    #[test]
    fn test_get_active_accounts() {
        let mut manager = AccountManager::new();

        let account1 = create_test_account("a1", AccountPurpose::Scalp);
        let account2 = create_test_account("a2", AccountPurpose::Swing);

        manager.register_account(account1).unwrap();
        manager.register_account(account2).unwrap();

        assert_eq!(manager.get_active_accounts().len(), 2);

        manager.deactivate_account("a1").unwrap();
        assert_eq!(manager.get_active_accounts().len(), 1);

        manager.activate_account("a1").unwrap();
        assert_eq!(manager.get_active_accounts().len(), 2);
    }

    #[test]
    fn test_deactivate_nonexistent_account() {
        let mut manager = AccountManager::new();
        let result = manager.deactivate_account("nonexistent");
        assert_eq!(result.unwrap_err(), Error::AccountNotFound);
    }

    #[test]
    fn test_set_leverage() {
        let mut manager = AccountManager::new();
        let account = create_test_account("lev", AccountPurpose::Scalp);
        manager.register_account(account).unwrap();

        // Valid leverage
        manager.set_leverage("lev", 50.0).unwrap();
        assert_eq!(manager.get_account("lev").unwrap().current_leverage, 50.0);

        // Invalid: exceeds max for purpose
        let result = manager.set_leverage("lev", 150.0); // Scalp max is 100
        assert!(result.is_err());

        // Invalid: negative
        let result = manager.set_leverage("lev", -10.0);
        assert!(result.is_err());
    }

    #[test]
    fn test_set_capital_allocation() {
        let mut manager = AccountManager::new();
        let account = create_test_account("cap", AccountPurpose::Swing);
        manager.register_account(account).unwrap();

        manager.set_capital_allocation("cap", 0.5).unwrap();
        assert_eq!(
            manager.get_account("cap").unwrap().capital_allocation,
            0.5
        );

        let result = manager.set_capital_allocation("cap", 1.5);
        assert!(result.is_err());

        let result = manager.set_capital_allocation("cap", -0.1);
        assert!(result.is_err());
    }

    #[test]
    fn test_total_capital_allocated() {
        let mut manager = AccountManager::new();

        let mut acc1 = create_test_account("a1", AccountPurpose::Scalp);
        acc1.capital_allocation = 0.3;

        let mut acc2 = create_test_account("a2", AccountPurpose::Swing);
        acc2.capital_allocation = 0.5;

        let mut acc3 = create_test_account("a3", AccountPurpose::Position);
        acc3.capital_allocation = 0.2;

        manager.register_account(acc1).unwrap();
        manager.register_account(acc2).unwrap();
        manager.register_account(acc3).unwrap();

        assert_eq!(manager.total_capital_allocated(), 1.0);
    }

    #[test]
    fn test_remove_account() {
        let mut manager = AccountManager::new();
        let account = create_test_account("rem", AccountPurpose::Scalp);

        manager.register_account(account).unwrap();
        assert_eq!(manager.total_account_count(), 1);

        let removed = manager.remove_account("rem");
        assert!(removed.is_some());
        assert_eq!(manager.total_account_count(), 0);
    }

    #[test]
    fn test_account_exists() {
        let mut manager = AccountManager::new();
        let account = create_test_account("exist", AccountPurpose::Scalp);

        manager.register_account(account).unwrap();
        assert!(manager.account_exists("exist"));
        assert!(!manager.account_exists("nonexistent"));
    }

    #[test]
    fn test_get_account_summary() {
        let mut manager = AccountManager::new();
        let account = create_test_account("sum", AccountPurpose::Scalp);

        manager.register_account(account).unwrap();
        let summary = manager.get_account_summary("sum").unwrap();

        assert_eq!(summary.id, "sum");
        assert_eq!(summary.purpose, AccountPurpose::Scalp);
        assert_eq!(summary.capital_allocation, 0.2);
        assert!(summary.is_active);
    }

    #[test]
    fn test_get_all_account_summaries() {
        let mut manager = AccountManager::new();

        let accounts = vec![
            create_test_account("s1", AccountPurpose::Scalp),
            create_test_account("s2", AccountPurpose::Swing),
            create_test_account("s3", AccountPurpose::Position),
        ];

        for account in accounts {
            manager.register_account(account).unwrap();
        }

        let summaries = manager.get_all_account_summaries();
        assert_eq!(summaries.len(), 3);

        let scalp_summary = summaries.iter().find(|s| s.id == "s1").unwrap();
        assert_eq!(scalp_summary.purpose, AccountPurpose::Scalp);
    }

    #[test]
    fn test_set_max_position_size() {
        let mut manager = AccountManager::new();
        let account = create_test_account("pos", AccountPurpose::Scalp);
        manager.register_account(account).unwrap();

        manager.set_max_position_size("pos", 0.15).unwrap();
        assert_eq!(
            manager.get_account("pos").unwrap().max_position_size,
            0.15
        );

        // Invalid: too large
        let result = manager.set_max_position_size("pos", 0.6);
        assert!(result.is_err());

        // Invalid: negative
        let result = manager.set_max_position_size("pos", -0.1);
        assert!(result.is_err());
    }

    #[test]
    fn test_account_updated_timestamp() {
        let mut manager = AccountManager::new();
        let account = create_test_account("ts", AccountPurpose::Scalp);
        let original_ts = account.updated_at;

        manager.register_account(account).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(10));
        manager.set_leverage("ts", 50.0).unwrap();

        let updated_account = manager.get_account("ts").unwrap();
        assert!(updated_account.updated_at > original_ts);
    }

    #[test]
    fn test_active_account_count() {
        let mut manager = AccountManager::new();

        let acc1 = create_test_account("a1", AccountPurpose::Scalp);
        let acc2 = create_test_account("a2", AccountPurpose::Swing);

        manager.register_account(acc1).unwrap();
        manager.register_account(acc2).unwrap();

        assert_eq!(manager.active_account_count(), 2);

        manager.deactivate_account("a1").unwrap();
        assert_eq!(manager.active_account_count(), 1);
    }

    #[test]
    fn test_invalid_account_registration() {
        let mut manager = AccountManager::new();

        let mut account = TradingAccount::new(
            "invalid".to_string(),
            Protocol::Drift,
            "key".to_string(),
            AccountPurpose::Scalp,
        );
        account.capital_allocation = 1.5; // Invalid

        let result = manager.register_account(account);
        assert!(result.is_err());
    }

    #[test]
    fn test_multiple_protocols() {
        let mut manager = AccountManager::new();

        let mut drift_scalp = create_test_account("drift-scalp", AccountPurpose::Scalp);
        drift_scalp.protocol = Protocol::Drift;

        let mut hpl_scalp = create_test_account("hpl-scalp", AccountPurpose::Scalp);
        hpl_scalp.protocol = Protocol::Hyperliquid;

        manager.register_account(drift_scalp).unwrap();
        manager.register_account(hpl_scalp).unwrap();

        assert_eq!(
            manager.get_accounts_by_protocol(Protocol::Drift).len(),
            1
        );
        assert_eq!(
            manager.get_accounts_by_protocol(Protocol::Hyperliquid).len(),
            1
        );
    }
}
