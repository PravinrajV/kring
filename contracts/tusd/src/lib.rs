//! tUSD: the Kring test dollar. A plain NEP-141 token for testnet demos only.
//! It has no value and no owner. Anyone registered can mint up to 100,000 tUSD per call,
//! which is exactly why it must never be used outside a testnet demo.

use near_contract_standards::fungible_token::events::FtMint;
use near_contract_standards::fungible_token::metadata::{
    FungibleTokenMetadata, FungibleTokenMetadataProvider, FT_METADATA_SPEC,
};
use near_contract_standards::fungible_token::{
    FungibleToken, FungibleTokenCore, FungibleTokenResolver,
};
use near_contract_standards::storage_management::{
    StorageBalance, StorageBalanceBounds, StorageManagement,
};
use near_sdk::borsh::BorshSerialize;
use near_sdk::collections::LazyOption;
use near_sdk::json_types::U128;
use near_sdk::{
    env, log, near, require, AccountId, BorshStorageKey, NearToken, PanicOnDefault, PromiseOrValue,
};

/// 6 decimals, like USDC.
pub const DECIMALS: u8 = 6;
/// Max mint per call: 100,000 tUSD.
pub const MAX_MINT: u128 = 100_000 * 1_000_000;

#[derive(PanicOnDefault)]
#[near(contract_state)]
pub struct Contract {
    token: FungibleToken,
    metadata: LazyOption<FungibleTokenMetadata>,
}

#[derive(BorshSerialize, BorshStorageKey)]
#[borsh(crate = "near_sdk::borsh")]
enum StorageKey {
    FungibleToken,
    Metadata,
}

#[near]
impl Contract {
    #[init]
    pub fn new() -> Self {
        let metadata = FungibleTokenMetadata {
            spec: FT_METADATA_SPEC.to_string(),
            name: "Kring test dollar (testnet only, no value)".to_string(),
            symbol: "tUSD".to_string(),
            icon: None,
            reference: None,
            reference_hash: None,
            decimals: DECIMALS,
        };
        metadata.assert_valid();
        Self {
            token: FungibleToken::new(StorageKey::FungibleToken),
            metadata: LazyOption::new(StorageKey::Metadata, Some(&metadata)),
        }
    }

    /// Test faucet. The caller must already be registered (storage_deposit).
    pub fn mint(&mut self, amount: U128) -> U128 {
        let account = env::predecessor_account_id();
        require!(amount.0 > 0 && amount.0 <= MAX_MINT, "Mint 1 to 100,000 tUSD per call");
        require!(
            self.token.accounts.contains_key(&account),
            "Register first with storage_deposit"
        );
        self.token.internal_deposit(&account, amount.0);
        FtMint { owner_id: &account, amount, memo: Some("test faucet") }.emit();
        self.token.ft_balance_of(account)
    }
}

#[near]
impl FungibleTokenCore for Contract {
    #[payable]
    fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>) {
        self.token.ft_transfer(receiver_id, amount, memo)
    }

    #[payable]
    fn ft_transfer_call(
        &mut self,
        receiver_id: AccountId,
        amount: U128,
        memo: Option<String>,
        msg: String,
    ) -> PromiseOrValue<U128> {
        self.token.ft_transfer_call(receiver_id, amount, memo, msg)
    }

    fn ft_total_supply(&self) -> U128 {
        self.token.ft_total_supply()
    }

    fn ft_balance_of(&self, account_id: AccountId) -> U128 {
        self.token.ft_balance_of(account_id)
    }
}

#[near]
impl FungibleTokenResolver for Contract {
    #[private]
    fn ft_resolve_transfer(
        &mut self,
        sender_id: AccountId,
        receiver_id: AccountId,
        amount: U128,
    ) -> U128 {
        let (used_amount, burned_amount) =
            self.token.internal_ft_resolve_transfer(&sender_id, receiver_id, amount);
        if burned_amount > 0 {
            log!("Account @{} burned {}", sender_id, burned_amount);
        }
        used_amount.into()
    }
}

#[near]
impl StorageManagement for Contract {
    #[payable]
    fn storage_deposit(
        &mut self,
        account_id: Option<AccountId>,
        registration_only: Option<bool>,
    ) -> StorageBalance {
        self.token.storage_deposit(account_id, registration_only)
    }

    #[payable]
    fn storage_withdraw(&mut self, amount: Option<NearToken>) -> StorageBalance {
        self.token.storage_withdraw(amount)
    }

    #[payable]
    fn storage_unregister(&mut self, force: Option<bool>) -> bool {
        if let Some((account_id, balance)) = self.token.internal_storage_unregister(force) {
            log!("Closed @{} with {}", account_id, balance);
            true
        } else {
            false
        }
    }

    fn storage_balance_bounds(&self) -> StorageBalanceBounds {
        self.token.storage_balance_bounds()
    }

    fn storage_balance_of(&self, account_id: AccountId) -> Option<StorageBalance> {
        self.token.storage_balance_of(account_id)
    }
}

#[near]
impl FungibleTokenMetadataProvider for Contract {
    fn ft_metadata(&self) -> FungibleTokenMetadata {
        self.metadata.get().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::{accounts, VMContextBuilder};
    use near_sdk::testing_env;

    fn ctx(who: AccountId, deposit: NearToken) {
        testing_env!(VMContextBuilder::new()
            .predecessor_account_id(who)
            .attached_deposit(deposit)
            .build());
    }

    #[test]
    fn mint_needs_registration_and_cap() {
        ctx(accounts(0), NearToken::from_near(1));
        let mut c = Contract::new();
        c.storage_deposit(None, None);
        ctx(accounts(0), NearToken::from_yoctonear(0));
        assert_eq!(c.mint(U128(5_000_000)).0, 5_000_000);
        assert_eq!(c.ft_total_supply().0, 5_000_000);
    }

    #[test]
    #[should_panic(expected = "Register first")]
    fn mint_unregistered_fails() {
        ctx(accounts(1), NearToken::from_yoctonear(0));
        let mut c = Contract::new();
        c.mint(U128(1));
    }

    #[test]
    #[should_panic(expected = "Mint 1 to 100,000")]
    fn mint_over_cap_fails() {
        ctx(accounts(0), NearToken::from_near(1));
        let mut c = Contract::new();
        c.storage_deposit(None, None);
        c.mint(U128(MAX_MINT + 1));
    }
}
