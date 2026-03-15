/// Integration tests for the trading system
use tradingbots_fun::*;

#[test]
fn test_complete_account_lifecycle() {
    let mut manager = AccountManager::new();

    // Create account
    let mut account = TradingAccount::new(
        "integration-test-1".to_string(),
        Protocol::Drift,
        "test_key_123".to_string(),
        AccountPurpose::Scalp,
    );
    account.capital_allocation = 0.25;

    // Register
    let id = manager.register_account(account).unwrap();
    assert_eq!(id, "integration-test-1");
    assert_eq!(manager.total_account_count(), 1);

    // Retrieve and verify
    let retrieved = manager.get_account(&id).unwrap();
    assert_eq!(retrieved.purpose, AccountPurpose::Scalp);
    assert_eq!(retrieved.capital_allocation, 0.25);
    assert!(retrieved.is_active);

    // Modify
    manager.set_leverage(&id, 50.0).unwrap();
    assert_eq!(
        manager.get_account(&id).unwrap().current_leverage,
        50.0
    );

    // Deactivate
    manager.deactivate_account(&id).unwrap();
    assert!(!manager.get_account(&id).unwrap().is_active);
    assert_eq!(manager.active_account_count(), 0);

    // Reactivate
    manager.activate_account(&id).unwrap();
    assert!(manager.get_account(&id).unwrap().is_active);
    assert_eq!(manager.active_account_count(), 1);

    // Remove
    let removed = manager.remove_account(&id);
    assert!(removed.is_some());
    assert_eq!(manager.total_account_count(), 0);
}

#[test]
fn test_multi_protocol_account_management() {
    let mut manager = AccountManager::new();

    // Drift accounts
    let drift_scalp = TradingAccount::new(
        "drift-scalp".to_string(),
        Protocol::Drift,
        "drift_key_1".to_string(),
        AccountPurpose::Scalp,
    );

    let drift_swing = TradingAccount::new(
        "drift-swing".to_string(),
        Protocol::Drift,
        "drift_key_2".to_string(),
        AccountPurpose::Swing,
    );

    // Hyperliquid accounts
    let mut hpl_hft = TradingAccount::new(
        "hpl-hft".to_string(),
        Protocol::Hyperliquid,
        "hpl_key_1".to_string(),
        AccountPurpose::Scalp,
    );
    hpl_hft.current_leverage = 50.0; // HPL allows different max

    // Register all
    manager.register_account(drift_scalp).unwrap();
    manager.register_account(drift_swing).unwrap();
    manager.register_account(hpl_hft).unwrap();

    // Query by protocol
    assert_eq!(
        manager.get_accounts_by_protocol(Protocol::Drift).len(),
        2
    );
    assert_eq!(
        manager.get_accounts_by_protocol(Protocol::Hyperliquid).len(),
        1
    );

    // Query by purpose
    assert_eq!(
        manager.get_accounts_by_purpose(AccountPurpose::Scalp).len(),
        2
    );
    assert_eq!(
        manager.get_accounts_by_purpose(AccountPurpose::Swing).len(),
        1
    );
}

#[test]
fn test_capital_allocation_validation() {
    let mut manager = AccountManager::new();

    let mut acc1 = TradingAccount::new(
        "a1".to_string(),
        Protocol::Drift,
        "k1".to_string(),
        AccountPurpose::Scalp,
    );
    acc1.capital_allocation = 0.4;

    let mut acc2 = TradingAccount::new(
        "a2".to_string(),
        Protocol::Drift,
        "k2".to_string(),
        AccountPurpose::Swing,
    );
    acc2.capital_allocation = 0.3;

    let mut acc3 = TradingAccount::new(
        "a3".to_string(),
        Protocol::Drift,
        "k3".to_string(),
        AccountPurpose::Position,
    );
    acc3.capital_allocation = 0.3;

    manager.register_account(acc1).unwrap();
    manager.register_account(acc2).unwrap();
    manager.register_account(acc3).unwrap();

    // Total should be exactly 1.0
    assert_eq!(manager.total_capital_allocated(), 1.0);

    // Try to rebalance to new valid distribution
    manager.set_capital_allocation("a1", 0.5).unwrap();
    manager.set_capital_allocation("a2", 0.25).unwrap();
    manager.set_capital_allocation("a3", 0.25).unwrap();

    assert_eq!(manager.total_capital_allocated(), 1.0);
}

#[test]
fn test_leverage_constraints_by_purpose() {
    let mut manager = AccountManager::new();

    let purposes = [
        (AccountPurpose::Scalp, 100.0),
        (AccountPurpose::Swing, 20.0),
        (AccountPurpose::Position, 10.0),
        (AccountPurpose::Hedge, 5.0),
        (AccountPurpose::Reserve, 0.0),
    ];

    for (i, (purpose, max_leverage)) in purposes.iter().enumerate() {
        let mut account = TradingAccount::new(
            format!("acc-{}", i),
            Protocol::Drift,
            format!("key-{}", i),
            *purpose,
        );
        account.capital_allocation = 0.2;

        manager.register_account(account).unwrap();
        let id = format!("acc-{}", i);

        // Should succeed at max
        assert!(manager.set_leverage(&id, *max_leverage).is_ok());

        // Should fail above max
        if *max_leverage > 0.0 {
            assert!(manager.set_leverage(&id, max_leverage + 1.0).is_err());
        }
    }
}

#[test]
fn test_account_summary_generation() {
    let mut manager = AccountManager::new();

    let accounts = vec![
        ("s1", AccountPurpose::Scalp),
        ("s2", AccountPurpose::Swing),
        ("s3", AccountPurpose::Position),
    ];

    for (id, purpose) in accounts {
        let mut account = TradingAccount::new(
            id.to_string(),
            Protocol::Drift,
            format!("key_{}", id),
            purpose,
        );
        account.capital_allocation = 0.33;
        manager.register_account(account).unwrap();
    }

    let summaries = manager.get_all_account_summaries();
    assert_eq!(summaries.len(), 3);

    for summary in &summaries {
        assert!(summary.is_active);
        assert_eq!(summary.protocol, Protocol::Drift);
        assert!(summary.capital_allocation > 0.0);
    }
}

#[test]
fn test_error_handling() {
    let mut manager = AccountManager::new();

    let account = TradingAccount::new(
        "test".to_string(),
        Protocol::Drift,
        "key".to_string(),
        AccountPurpose::Scalp,
    );

    // Register successfully
    assert!(manager.register_account(account).is_ok());

    // Try duplicate
    let duplicate = TradingAccount::new(
        "test".to_string(),
        Protocol::Drift,
        "key2".to_string(),
        AccountPurpose::Swing,
    );
    assert_eq!(
        manager.register_account(duplicate).unwrap_err(),
        Error::DuplicateAccount
    );

    // Try non-existent
    assert_eq!(
        manager.deactivate_account("nonexistent").unwrap_err(),
        Error::AccountNotFound
    );

    // Try invalid leverage
    assert!(manager.set_leverage("test", 200.0).is_err()); // Max 100 for Scalp

    // Try invalid capital
    assert!(manager.set_capital_allocation("test", 1.5).is_err());
}

#[test]
fn test_high_throughput_account_creation() {
    let mut manager = AccountManager::new();

    // Create 100 accounts
    for i in 0..100 {
        let mut account = TradingAccount::new(
            format!("high-throughput-{}", i),
            Protocol::Drift,
            format!("key-{}", i),
            if i % 5 == 0 {
                AccountPurpose::Scalp
            } else if i % 5 == 1 {
                AccountPurpose::Swing
            } else if i % 5 == 2 {
                AccountPurpose::Position
            } else if i % 5 == 3 {
                AccountPurpose::Hedge
            } else {
                AccountPurpose::Reserve
            },
        );
        account.capital_allocation = 0.01; // 1% each = 100%

        assert!(manager.register_account(account).is_ok());
    }

    assert_eq!(manager.total_account_count(), 100);
    assert_eq!(manager.active_account_count(), 100);

    // Verify counts by purpose
    assert_eq!(
        manager.get_accounts_by_purpose(AccountPurpose::Scalp).len(),
        20
    );
    assert_eq!(
        manager.get_accounts_by_purpose(AccountPurpose::Swing).len(),
        20
    );
    assert_eq!(
        manager.get_accounts_by_purpose(AccountPurpose::Position).len(),
        20
    );
    assert_eq!(
        manager.get_accounts_by_purpose(AccountPurpose::Hedge).len(),
        20
    );
    assert_eq!(
        manager.get_accounts_by_purpose(AccountPurpose::Reserve).len(),
        20
    );
}

#[test]
fn test_account_filtering_combinations() {
    let mut manager = AccountManager::new();

    // Create accounts with different combinations
    let mut drift_scalp = TradingAccount::new(
        "drift-scalp".to_string(),
        Protocol::Drift,
        "key1".to_string(),
        AccountPurpose::Scalp,
    );
    drift_scalp.capital_allocation = 0.2;

    let mut drift_swing = TradingAccount::new(
        "drift-swing".to_string(),
        Protocol::Drift,
        "key2".to_string(),
        AccountPurpose::Swing,
    );
    drift_swing.capital_allocation = 0.2;

    let mut hpl_scalp = TradingAccount::new(
        "hpl-scalp".to_string(),
        Protocol::Hyperliquid,
        "key3".to_string(),
        AccountPurpose::Scalp,
    );
    hpl_scalp.capital_allocation = 0.3;

    let mut hpl_position = TradingAccount::new(
        "hpl-position".to_string(),
        Protocol::Hyperliquid,
        "key4".to_string(),
        AccountPurpose::Position,
    );
    hpl_position.capital_allocation = 0.3;

    manager.register_account(drift_scalp).unwrap();
    manager.register_account(drift_swing).unwrap();
    manager.register_account(hpl_scalp).unwrap();
    manager.register_account(hpl_position).unwrap();

    // Filter by protocol
    assert_eq!(
        manager.get_accounts_by_protocol(Protocol::Drift).len(),
        2
    );
    assert_eq!(
        manager.get_accounts_by_protocol(Protocol::Hyperliquid).len(),
        2
    );

    // Filter by purpose
    assert_eq!(
        manager.get_accounts_by_purpose(AccountPurpose::Scalp).len(),
        2
    );
    assert_eq!(
        manager.get_accounts_by_purpose(AccountPurpose::Swing).len(),
        1
    );
    assert_eq!(
        manager.get_accounts_by_purpose(AccountPurpose::Position).len(),
        1
    );

    // Filter active
    assert_eq!(manager.get_active_accounts().len(), 4);

    manager.deactivate_account("drift-scalp").unwrap();
    assert_eq!(manager.get_active_accounts().len(), 3);
}
