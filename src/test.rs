#![cfg(test)]
extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, EnvTestConfig, Ledger},
    token, Address, Env, Error as SdkError, InvokeError, String,
};

fn entorno() -> Env {
    // Sin esto, el SDK escribe un JSON enorme por cada prueba.
    Env::new_with_config(EnvTestConfig {
        capture_snapshot_at_drop: false,
    })
}

fn error_de<T: core::fmt::Debug>(resultado: Result<T, Result<SdkError, InvokeError>>) -> SdkError {
    resultado
        .expect_err("esta llamada debia fallar")
        .expect("el fallo debia ser un error del contrato")
}

fn producto(env: &Env, texto: &str) -> String {
    String::from_str(env, texto)
}

struct Mundo<'a> {
    env: Env,
    cliente: SenaFeriaClient<'a>,
    token: token::Client<'a>,
    comprador: Address,
    productor: Address,
}

fn mundo<'a>() -> Mundo<'a> {
    let env = entorno();
    env.mock_all_auths();
    env.ledger().with_mut(|ledger| {
        ledger.sequence_number = 1_000;
    });

    let contrato_id = env.register(SenaFeria, ());
    let cliente = SenaFeriaClient::new(&env, &contrato_id);

    let admin = Address::generate(&env);
    let token_id = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token = token::Client::new(&env, &token_id);
    let admin_token = token::StellarAssetClient::new(&env, &token_id);

    let comprador = Address::generate(&env);
    let productor = Address::generate(&env);
    admin_token.mint(&comprador, &10_000_0000);

    Mundo {
        env,
        cliente,
        token,
        comprador,
        productor,
    }
}

fn apartar_queso(mundo: &Mundo, plazo: u32) -> u32 {
    mundo.cliente.apartar(
        &mundo.comprador,
        &mundo.productor,
        &mundo.token.address,
        &2_500_0000,
        &producto(&mundo.env, "1 kg de queso Turrialba"),
        &plazo,
    )
}

#[test]
fn maria_aparta_el_queso_y_carlos_confirma_la_entrega() {
    let mundo = mundo();
    let id = apartar_queso(&mundo, 100);

    assert_eq!(mundo.token.balance(&mundo.comprador), 7_500_0000);
    assert_eq!(mundo.token.balance(&mundo.cliente.address), 2_500_0000);
    assert_eq!(mundo.token.balance(&mundo.productor), 0);

    let sena = mundo.cliente.consultar(&id);
    assert_eq!(sena.estado, Estado::Apartada);
    assert_eq!(sena.vence_en, 1_100);
    assert_eq!(
        sena.producto,
        producto(&mundo.env, "1 kg de queso Turrialba")
    );

    mundo.cliente.confirmar_entrega(&mundo.productor, &id);

    assert_eq!(mundo.token.balance(&mundo.cliente.address), 0);
    assert_eq!(mundo.token.balance(&mundo.productor), 2_500_0000);
    assert_eq!(mundo.cliente.consultar(&id).estado, Estado::Entregada);
}

#[test]
fn el_productor_puede_devolver_la_sena() {
    let mundo = mundo();
    let id = apartar_queso(&mundo, 100);

    mundo.cliente.devolver(&mundo.productor, &id);

    assert_eq!(mundo.token.balance(&mundo.comprador), 10_000_0000);
    assert_eq!(mundo.token.balance(&mundo.cliente.address), 0);
    assert_eq!(mundo.token.balance(&mundo.productor), 0);
    assert_eq!(mundo.cliente.consultar(&id).estado, Estado::Devuelta);
}

#[test]
fn vencida_la_sena_el_comprador_la_reclama() {
    let mundo = mundo();
    let id = apartar_queso(&mundo, 50);

    mundo.env.ledger().with_mut(|ledger| {
        ledger.sequence_number = 1_051;
    });

    mundo.cliente.reclamar_vencida(&mundo.comprador, &id);

    assert_eq!(mundo.token.balance(&mundo.comprador), 10_000_0000);
    assert_eq!(mundo.token.balance(&mundo.cliente.address), 0);
    assert_eq!(mundo.cliente.consultar(&id).estado, Estado::Devuelta);
}

#[test]
fn en_el_ledger_del_vencimiento_el_productor_todavia_puede_cobrar() {
    let mundo = mundo();
    let id = apartar_queso(&mundo, 50);

    mundo.env.ledger().with_mut(|ledger| {
        ledger.sequence_number = 1_050;
    });

    mundo.cliente.confirmar_entrega(&mundo.productor, &id);
    assert_eq!(mundo.token.balance(&mundo.productor), 2_500_0000);
}

#[test]
fn dos_senas_no_se_mezclan() {
    let mundo = mundo();
    let queso = apartar_queso(&mundo, 100);
    let miel = mundo.cliente.apartar(
        &mundo.comprador,
        &mundo.productor,
        &mundo.token.address,
        &1_000_0000,
        &producto(&mundo.env, "frasco de miel de 350 ml"),
        &80,
    );

    assert_eq!(queso, 1);
    assert_eq!(miel, 2);
    assert_eq!(mundo.token.balance(&mundo.cliente.address), 3_500_0000);

    mundo.cliente.confirmar_entrega(&mundo.productor, &queso);

    assert_eq!(mundo.token.balance(&mundo.productor), 2_500_0000);
    assert_eq!(mundo.token.balance(&mundo.cliente.address), 1_000_0000);
    assert_eq!(mundo.cliente.consultar(&miel).estado, Estado::Apartada);
    assert_eq!(
        mundo.cliente.consultar(&miel).producto,
        producto(&mundo.env, "frasco de miel de 350 ml")
    );
}

#[test]
fn no_se_puede_reclamar_antes_de_tiempo() {
    let mundo = mundo();
    let id = apartar_queso(&mundo, 50);
    let error = error_de(mundo.cliente.try_reclamar_vencida(&mundo.comprador, &id));
    assert_eq!(error, Error::AunNoVence.into());
    assert_eq!(mundo.token.balance(&mundo.cliente.address), 2_500_0000);
}

#[test]
fn el_productor_no_cobra_si_ya_vencio() {
    let mundo = mundo();
    let id = apartar_queso(&mundo, 50);
    mundo.env.ledger().with_mut(|ledger| {
        ledger.sequence_number = 1_051;
    });

    let error = error_de(mundo.cliente.try_confirmar_entrega(&mundo.productor, &id));
    assert_eq!(error, Error::YaVencio.into());
    assert_eq!(mundo.token.balance(&mundo.productor), 0);
}

#[test]
fn otra_persona_no_confirma_ni_devuelve_ni_reclama() {
    let mundo = mundo();
    let id = apartar_queso(&mundo, 50);
    let intruso = Address::generate(&mundo.env);

    assert_eq!(
        error_de(mundo.cliente.try_confirmar_entrega(&intruso, &id)),
        Error::NoEsProductor.into()
    );
    assert_eq!(
        error_de(mundo.cliente.try_devolver(&intruso, &id)),
        Error::NoEsProductor.into()
    );
    assert_eq!(
        error_de(mundo.cliente.try_reclamar_vencida(&intruso, &id)),
        Error::NoEsComprador.into()
    );
    assert_eq!(mundo.cliente.consultar(&id).estado, Estado::Apartada);
}

#[test]
fn no_se_puede_cerrar_dos_veces() {
    let mundo = mundo();
    let id = apartar_queso(&mundo, 50);
    mundo.cliente.confirmar_entrega(&mundo.productor, &id);

    assert_eq!(
        error_de(mundo.cliente.try_confirmar_entrega(&mundo.productor, &id)),
        Error::YaCerrada.into()
    );
    assert_eq!(
        error_de(mundo.cliente.try_devolver(&mundo.productor, &id)),
        Error::YaCerrada.into()
    );
    assert_eq!(mundo.token.balance(&mundo.productor), 2_500_0000);
    assert_eq!(mundo.token.balance(&mundo.comprador), 7_500_0000);
}

#[test]
fn rechaza_datos_invalidos_y_no_mueve_dinero() {
    let mundo = mundo();
    let antes = mundo.token.balance(&mundo.comprador);

    assert_eq!(
        error_de(mundo.cliente.try_apartar(
            &mundo.comprador,
            &mundo.productor,
            &mundo.token.address,
            &0,
            &producto(&mundo.env, "huevos de gallina"),
            &10,
        )),
        Error::MontoInvalido.into()
    );
    assert_eq!(
        error_de(mundo.cliente.try_apartar(
            &mundo.comprador,
            &mundo.comprador,
            &mundo.token.address,
            &100,
            &producto(&mundo.env, "huevos de gallina"),
            &10,
        )),
        Error::MismaPersona.into()
    );
    assert_eq!(
        error_de(mundo.cliente.try_apartar(
            &mundo.comprador,
            &mundo.productor,
            &mundo.token.address,
            &100,
            &producto(&mundo.env, "huevos de gallina"),
            &0,
        )),
        Error::PlazoInvalido.into()
    );
    assert_eq!(
        error_de(mundo.cliente.try_apartar(
            &mundo.comprador,
            &mundo.productor,
            &mundo.token.address,
            &100,
            &producto(&mundo.env, ""),
            &10,
        )),
        Error::DescripcionInvalida.into()
    );
    assert_eq!(
        error_de(mundo.cliente.try_consultar(&1)),
        Error::NoExiste.into()
    );
    assert_eq!(mundo.token.balance(&mundo.comprador), antes);
}

#[test]
#[should_panic(expected = "HostError: Error(Auth, InvalidAction)")]
fn sin_la_firma_del_comprador_no_se_aparta() {
    let env = entorno();
    let contrato_id = env.register(SenaFeria, ());
    let cliente = SenaFeriaClient::new(&env, &contrato_id);
    let comprador = Address::generate(&env);
    let productor = Address::generate(&env);
    let token = Address::generate(&env);

    // No hay mock_all_auths: apartar exige la firma del comprador y se detiene ahí.
    cliente.apartar(
        &comprador,
        &productor,
        &token,
        &500,
        &producto(&env, "manojo de culantro"),
        &10,
    );
}
