#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, EnvTestConfig, Ledger},
    token, Address, Env, Error as SdkError, InvokeError, String,
};

fn test_env() -> Env {
    // Otherwise the SDK writes a large JSON file for every test.
    Env::new_with_config(EnvTestConfig {
        capture_snapshot_at_drop: false,
    })
}

fn contract_error<T: core::fmt::Debug>(
    result: Result<T, Result<SdkError, InvokeError>>,
) -> SdkError {
    result
        .expect_err("this call should fail")
        .expect("the failure should be a contract error")
}

fn product(env: &Env, text: &str) -> String {
    String::from_str(env, text)
}

struct World<'a> {
    env: Env,
    client: FairDepositClient<'a>,
    token: token::Client<'a>,
    buyer: Address,
    producer: Address,
}

fn world<'a>() -> World<'a> {
    let env = test_env();
    env.mock_all_auths();
    env.ledger().with_mut(|ledger| {
        ledger.sequence_number = 1_000;
    });

    let contract_id = env.register(FairDeposit, ());
    let client = FairDepositClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token = token::Client::new(&env, &token_id);
    let token_admin = token::StellarAssetClient::new(&env, &token_id);

    let buyer = Address::generate(&env);
    let producer = Address::generate(&env);
    token_admin.mint(&buyer, &10_000_0000);

    World {
        env,
        client,
        token,
        buyer,
        producer,
    }
}

fn reserve_cheese(world: &World, duration: u32) -> u32 {
    world.client.reserve(
        &world.buyer,
        &world.producer,
        &world.token.address,
        &2_500_0000,
        &product(&world.env, "1 kg de queso Turrialba"),
        &duration,
    )
}

#[test]
fn buyer_reserves_cheese_and_producer_confirms_delivery() {
    let world = world();
    let id = reserve_cheese(&world, 100);

    assert_eq!(world.token.balance(&world.buyer), 7_500_0000);
    assert_eq!(world.token.balance(&world.client.address), 2_500_0000);
    assert_eq!(world.token.balance(&world.producer), 0);

    let deposit = world.client.get_deposit(&id);
    assert_eq!(deposit.status, Status::Held);
    assert_eq!(deposit.expires_at, 1_100);
    assert_eq!(
        deposit.product,
        product(&world.env, "1 kg de queso Turrialba")
    );

    world.client.confirm_delivery(&world.producer, &id);

    assert_eq!(world.token.balance(&world.client.address), 0);
    assert_eq!(world.token.balance(&world.producer), 2_500_0000);
    assert_eq!(world.client.get_deposit(&id).status, Status::Delivered);
}

#[test]
fn producer_can_refund_the_deposit() {
    let world = world();
    let id = reserve_cheese(&world, 100);

    world.client.refund(&world.producer, &id);

    assert_eq!(world.token.balance(&world.buyer), 10_000_0000);
    assert_eq!(world.token.balance(&world.client.address), 0);
    assert_eq!(world.token.balance(&world.producer), 0);
    assert_eq!(world.client.get_deposit(&id).status, Status::Refunded);
}

#[test]
fn buyer_claims_the_deposit_after_it_expires() {
    let world = world();
    let id = reserve_cheese(&world, 50);

    world.env.ledger().with_mut(|ledger| {
        ledger.sequence_number = 1_051;
    });

    world.client.claim_expired(&world.buyer, &id);

    assert_eq!(world.token.balance(&world.buyer), 10_000_0000);
    assert_eq!(world.token.balance(&world.client.address), 0);
    assert_eq!(world.client.get_deposit(&id).status, Status::Refunded);
}

#[test]
fn producer_can_still_be_paid_on_the_expiry_ledger() {
    let world = world();
    let id = reserve_cheese(&world, 50);

    world.env.ledger().with_mut(|ledger| {
        ledger.sequence_number = 1_050;
    });

    world.client.confirm_delivery(&world.producer, &id);
    assert_eq!(world.token.balance(&world.producer), 2_500_0000);
}

#[test]
fn two_deposits_do_not_share_funds() {
    let world = world();
    let cheese = reserve_cheese(&world, 100);
    let honey = world.client.reserve(
        &world.buyer,
        &world.producer,
        &world.token.address,
        &1_000_0000,
        &product(&world.env, "frasco de miel de 350 ml"),
        &80,
    );

    assert_eq!(cheese, 1);
    assert_eq!(honey, 2);
    assert_eq!(world.token.balance(&world.client.address), 3_500_0000);

    world.client.confirm_delivery(&world.producer, &cheese);

    assert_eq!(world.token.balance(&world.producer), 2_500_0000);
    assert_eq!(world.token.balance(&world.client.address), 1_000_0000);
    assert_eq!(world.client.get_deposit(&honey).status, Status::Held);
    assert_eq!(
        world.client.get_deposit(&honey).product,
        product(&world.env, "frasco de miel de 350 ml")
    );
}

#[test]
fn buyer_cannot_claim_before_the_deadline() {
    let world = world();
    let id = reserve_cheese(&world, 50);
    let error = contract_error(world.client.try_claim_expired(&world.buyer, &id));
    assert_eq!(error, Error::NotYetExpired.into());
    assert_eq!(world.token.balance(&world.client.address), 2_500_0000);
}

#[test]
fn producer_cannot_collect_after_expiry() {
    let world = world();
    let id = reserve_cheese(&world, 50);
    world.env.ledger().with_mut(|ledger| {
        ledger.sequence_number = 1_051;
    });

    let error = contract_error(world.client.try_confirm_delivery(&world.producer, &id));
    assert_eq!(error, Error::AlreadyExpired.into());
    assert_eq!(world.token.balance(&world.producer), 0);
}

#[test]
fn a_third_party_cannot_close_the_deposit() {
    let world = world();
    let id = reserve_cheese(&world, 50);
    let stranger = Address::generate(&world.env);

    assert_eq!(
        contract_error(world.client.try_confirm_delivery(&stranger, &id)),
        Error::NotProducer.into()
    );
    assert_eq!(
        contract_error(world.client.try_refund(&stranger, &id)),
        Error::NotProducer.into()
    );
    assert_eq!(
        contract_error(world.client.try_claim_expired(&stranger, &id)),
        Error::NotBuyer.into()
    );
    assert_eq!(world.client.get_deposit(&id).status, Status::Held);
}

#[test]
fn a_deposit_cannot_be_closed_twice() {
    let world = world();
    let id = reserve_cheese(&world, 50);
    world.client.confirm_delivery(&world.producer, &id);

    assert_eq!(
        contract_error(world.client.try_confirm_delivery(&world.producer, &id)),
        Error::AlreadyClosed.into()
    );
    assert_eq!(
        contract_error(world.client.try_refund(&world.producer, &id)),
        Error::AlreadyClosed.into()
    );
    assert_eq!(world.token.balance(&world.producer), 2_500_0000);
    assert_eq!(world.token.balance(&world.buyer), 7_500_0000);
}

#[test]
fn invalid_input_is_rejected_and_moves_no_funds() {
    let world = world();
    let before = world.token.balance(&world.buyer);

    assert_eq!(
        contract_error(world.client.try_reserve(
            &world.buyer,
            &world.producer,
            &world.token.address,
            &0,
            &product(&world.env, "huevos de gallina"),
            &10,
        )),
        Error::InvalidAmount.into()
    );
    assert_eq!(
        contract_error(world.client.try_reserve(
            &world.buyer,
            &world.buyer,
            &world.token.address,
            &100,
            &product(&world.env, "huevos de gallina"),
            &10,
        )),
        Error::SameParty.into()
    );
    assert_eq!(
        contract_error(world.client.try_reserve(
            &world.buyer,
            &world.producer,
            &world.token.address,
            &100,
            &product(&world.env, "huevos de gallina"),
            &0,
        )),
        Error::InvalidDeadline.into()
    );
    assert_eq!(
        contract_error(world.client.try_reserve(
            &world.buyer,
            &world.producer,
            &world.token.address,
            &100,
            &product(&world.env, ""),
            &10,
        )),
        Error::InvalidDescription.into()
    );
    assert_eq!(
        contract_error(world.client.try_get_deposit(&1)),
        Error::NotFound.into()
    );
    assert_eq!(world.token.balance(&world.buyer), before);
}

#[test]
#[should_panic(expected = "HostError: Error(Auth, InvalidAction)")]
fn reserve_fails_without_the_buyers_signature() {
    let env = test_env();
    let contract_id = env.register(FairDeposit, ());
    let client = FairDepositClient::new(&env, &contract_id);
    let buyer = Address::generate(&env);
    let producer = Address::generate(&env);
    let token = Address::generate(&env);

    // No mock_all_auths: reserve requires the buyer's signature and stops there.
    client.reserve(
        &buyer,
        &producer,
        &token,
        &500,
        &product(&env, "manojo de culantro"),
        &10,
    );
}
