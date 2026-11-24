//! Farmers-market deposit.
//!
//! The buyer reserves a product and the contract holds the deposit.
//! The funds leave only one of these ways:
//! - the producer confirms delivery and keeps the deposit;
//! - the producer refunds it;
//! - the deadline passes and the buyer claims it back.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, token,
    Address, Env, String,
};

const MAX_DESCRIPTION: u32 = 64;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    InvalidAmount = 1,
    SameParty = 2,
    InvalidDeadline = 3,
    InvalidDescription = 4,
    NotFound = 5,
    AlreadyClosed = 6,
    NotProducer = 7,
    NotBuyer = 8,
    NotYetExpired = 9,
    AlreadyExpired = 10,
}

/// Held: the deposit is still inside the contract.
/// Delivered: the producer confirmed and already received the funds.
/// Refunded: the funds went back to the buyer.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Status {
    Held,
    Delivered,
    Refunded,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Deposit {
    pub buyer: Address,
    pub producer: Address,
    pub token: Address,
    pub amount: i128,
    pub expires_at: u32,
    pub status: Status,
    pub product: String,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Deposit(u32),
    Counter,
}

#[contract]
pub struct FairDeposit;

#[contractimpl]
impl FairDeposit {
    /// The buyer locks the deposit. Returns the deposit id.
    ///
    /// `duration_ledgers` is how many ledgers the reservation lasts,
    /// counted from the current ledger. On testnet a ledger is about 5 seconds.
    pub fn reserve(
        env: Env,
        buyer: Address,
        producer: Address,
        token: Address,
        amount: i128,
        product: String,
        duration_ledgers: u32,
    ) -> u32 {
        buyer.require_auth();

        if amount <= 0 {
            panic_with_error!(&env, Error::InvalidAmount);
        }
        if buyer == producer {
            panic_with_error!(&env, Error::SameParty);
        }
        if duration_ledgers == 0 {
            panic_with_error!(&env, Error::InvalidDeadline);
        }
        let length = product.len();
        if length == 0 || length > MAX_DESCRIPTION {
            panic_with_error!(&env, Error::InvalidDescription);
        }

        let expires_at = env
            .ledger()
            .sequence()
            .checked_add(duration_ledgers)
            .unwrap_or_else(|| panic_with_error!(&env, Error::InvalidDeadline));

        // Take the funds first. If the transfer fails, no empty deposit is stored.
        token::Client::new(&env, &token).transfer(&buyer, &env.current_contract_address(), &amount);

        let id = next_id(&env);
        let deposit = Deposit {
            buyer: buyer.clone(),
            producer,
            token,
            amount,
            expires_at,
            status: Status::Held,
            product,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Deposit(id), &deposit);

        env.events()
            .publish((symbol_short!("reserved"), id), amount);

        id
    }

    /// The producer confirms delivery and withdraws the deposit.
    /// This is allowed only while the deadline is still open.
    pub fn confirm_delivery(env: Env, producer: Address, id: u32) {
        producer.require_auth();

        let mut deposit = load(&env, id);
        require_open(&env, &deposit);
        if producer != deposit.producer {
            panic_with_error!(&env, Error::NotProducer);
        }
        if !is_open(&env, &deposit) {
            panic_with_error!(&env, Error::AlreadyExpired);
        }

        // Close the deposit before moving funds so it cannot be paid twice.
        deposit.status = Status::Delivered;
        env.storage()
            .persistent()
            .set(&DataKey::Deposit(id), &deposit);

        token::Client::new(&env, &deposit.token).transfer(
            &env.current_contract_address(),
            &deposit.producer,
            &deposit.amount,
        );

        env.events()
            .publish((symbol_short!("delivered"), id), deposit.amount);
    }

    /// The producer refunds the deposit. This works at any time
    /// while the deposit is still held, including after the deadline.
    pub fn refund(env: Env, producer: Address, id: u32) {
        producer.require_auth();

        let mut deposit = load(&env, id);
        require_open(&env, &deposit);
        if producer != deposit.producer {
            panic_with_error!(&env, Error::NotProducer);
        }

        deposit.status = Status::Refunded;
        env.storage()
            .persistent()
            .set(&DataKey::Deposit(id), &deposit);

        token::Client::new(&env, &deposit.token).transfer(
            &env.current_contract_address(),
            &deposit.buyer,
            &deposit.amount,
        );

        env.events()
            .publish((symbol_short!("refunded"), id), deposit.amount);
    }

    /// After the deadline, the buyer takes the deposit back if nobody closed it.
    pub fn claim_expired(env: Env, buyer: Address, id: u32) {
        buyer.require_auth();

        let mut deposit = load(&env, id);
        require_open(&env, &deposit);
        if buyer != deposit.buyer {
            panic_with_error!(&env, Error::NotBuyer);
        }
        if is_open(&env, &deposit) {
            panic_with_error!(&env, Error::NotYetExpired);
        }

        deposit.status = Status::Refunded;
        env.storage()
            .persistent()
            .set(&DataKey::Deposit(id), &deposit);

        token::Client::new(&env, &deposit.token).transfer(
            &env.current_contract_address(),
            &deposit.buyer,
            &deposit.amount,
        );

        env.events()
            .publish((symbol_short!("claimed"), id), deposit.amount);
    }

    /// Reads a deposit. Fails if that id does not exist.
    pub fn get_deposit(env: Env, id: u32) -> Deposit {
        load(&env, id)
    }
}

fn is_open(env: &Env, deposit: &Deposit) -> bool {
    env.ledger().sequence() <= deposit.expires_at
}

fn require_open(env: &Env, deposit: &Deposit) {
    if deposit.status != Status::Held {
        panic_with_error!(env, Error::AlreadyClosed);
    }
}

fn load(env: &Env, id: u32) -> Deposit {
    env.storage()
        .persistent()
        .get(&DataKey::Deposit(id))
        .unwrap_or_else(|| panic_with_error!(env, Error::NotFound))
}

fn next_id(env: &Env) -> u32 {
    let current: u32 = env.storage().instance().get(&DataKey::Counter).unwrap_or(0);
    let next = current + 1;
    env.storage().instance().set(&DataKey::Counter, &next);
    next
}

#[cfg(test)]
mod test;
