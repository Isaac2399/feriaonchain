//! Seña de la feria del agricultor.
//!
//! El comprador aparta un producto y el contrato guarda la seña.
//! El dinero sale solo por uno de estos caminos:
//! - el productor confirma la entrega y se queda con la seña;
//! - el productor la devuelve;
//! - se vence el plazo y el comprador la reclama.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, token,
    Address, Env, String,
};

const MAX_DESCRIPCION: u32 = 64;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    MontoInvalido = 1,
    MismaPersona = 2,
    PlazoInvalido = 3,
    DescripcionInvalida = 4,
    NoExiste = 5,
    YaCerrada = 6,
    NoEsProductor = 7,
    NoEsComprador = 8,
    AunNoVence = 9,
    YaVencio = 10,
}

/// Apartada: la seña sigue dentro del contrato.
/// Entregada: el productor confirmó y ya recibió el dinero.
/// Devuelta: el dinero volvió al comprador.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Estado {
    Apartada,
    Entregada,
    Devuelta,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sena {
    pub comprador: Address,
    pub productor: Address,
    pub token: Address,
    pub monto: i128,
    pub vence_en: u32,
    pub estado: Estado,
    pub producto: String,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Sena(u32),
    Contador,
}

#[contract]
pub struct SenaFeria;

#[contractimpl]
impl SenaFeria {
    /// El comprador deposita la seña. Devuelve el número de la seña.
    ///
    /// `plazo_ledgers` es cuántos ledgers dura el apartado, contados
    /// desde el ledger actual. En testnet un ledger dura unos 5 segundos.
    pub fn apartar(
        env: Env,
        comprador: Address,
        productor: Address,
        token: Address,
        monto: i128,
        producto: String,
        plazo_ledgers: u32,
    ) -> u32 {
        comprador.require_auth();

        if monto <= 0 {
            panic_with_error!(&env, Error::MontoInvalido);
        }
        if comprador == productor {
            panic_with_error!(&env, Error::MismaPersona);
        }
        if plazo_ledgers == 0 {
            panic_with_error!(&env, Error::PlazoInvalido);
        }
        let largo = producto.len();
        if largo == 0 || largo > MAX_DESCRIPCION {
            panic_with_error!(&env, Error::DescripcionInvalida);
        }

        let vence_en = env
            .ledger()
            .sequence()
            .checked_add(plazo_ledgers)
            .unwrap_or_else(|| panic_with_error!(&env, Error::PlazoInvalido));

        // Primero entra el dinero. Si el transfer falla, no queda una seña vacía.
        token::Client::new(&env, &token).transfer(
            &comprador,
            &env.current_contract_address(),
            &monto,
        );

        let id = siguiente_id(&env);
        let sena = Sena {
            comprador: comprador.clone(),
            productor,
            token,
            monto,
            vence_en,
            estado: Estado::Apartada,
            producto,
        };
        env.storage().persistent().set(&DataKey::Sena(id), &sena);

        env.events().publish((symbol_short!("apartada"), id), monto);

        id
    }

    /// El productor confirma que entregó el producto y retira la seña.
    /// Solo puede hacerlo mientras el plazo siga vigente.
    pub fn confirmar_entrega(env: Env, productor: Address, id: u32) {
        productor.require_auth();

        let mut sena = leer(&env, id);
        exigir_abierta(&env, &sena);
        if productor != sena.productor {
            panic_with_error!(&env, Error::NoEsProductor);
        }
        if !vigente(&env, &sena) {
            panic_with_error!(&env, Error::YaVencio);
        }

        // Se cierra antes de mover el dinero para no pagarlo dos veces.
        sena.estado = Estado::Entregada;
        env.storage().persistent().set(&DataKey::Sena(id), &sena);

        token::Client::new(&env, &sena.token).transfer(
            &env.current_contract_address(),
            &sena.productor,
            &sena.monto,
        );

        env.events()
            .publish((symbol_short!("entregada"), id), sena.monto);
    }

    /// El productor devuelve la seña. Puede hacerlo en cualquier momento
    /// mientras la seña siga apartada, incluso después del plazo.
    pub fn devolver(env: Env, productor: Address, id: u32) {
        productor.require_auth();

        let mut sena = leer(&env, id);
        exigir_abierta(&env, &sena);
        if productor != sena.productor {
            panic_with_error!(&env, Error::NoEsProductor);
        }

        sena.estado = Estado::Devuelta;
        env.storage().persistent().set(&DataKey::Sena(id), &sena);

        token::Client::new(&env, &sena.token).transfer(
            &env.current_contract_address(),
            &sena.comprador,
            &sena.monto,
        );

        env.events()
            .publish((symbol_short!("devuelta"), id), sena.monto);
    }

    /// Pasado el plazo, el comprador recupera la seña si nadie la cerró.
    pub fn reclamar_vencida(env: Env, comprador: Address, id: u32) {
        comprador.require_auth();

        let mut sena = leer(&env, id);
        exigir_abierta(&env, &sena);
        if comprador != sena.comprador {
            panic_with_error!(&env, Error::NoEsComprador);
        }
        if vigente(&env, &sena) {
            panic_with_error!(&env, Error::AunNoVence);
        }

        sena.estado = Estado::Devuelta;
        env.storage().persistent().set(&DataKey::Sena(id), &sena);

        token::Client::new(&env, &sena.token).transfer(
            &env.current_contract_address(),
            &sena.comprador,
            &sena.monto,
        );

        env.events()
            .publish((symbol_short!("reclamo"), id), sena.monto);
    }

    /// Lee una seña. Falla si ese número no existe.
    pub fn consultar(env: Env, id: u32) -> Sena {
        leer(&env, id)
    }
}

fn vigente(env: &Env, sena: &Sena) -> bool {
    env.ledger().sequence() <= sena.vence_en
}

fn exigir_abierta(env: &Env, sena: &Sena) {
    if sena.estado != Estado::Apartada {
        panic_with_error!(env, Error::YaCerrada);
    }
}

fn leer(env: &Env, id: u32) -> Sena {
    env.storage()
        .persistent()
        .get(&DataKey::Sena(id))
        .unwrap_or_else(|| panic_with_error!(env, Error::NoExiste))
}

fn siguiente_id(env: &Env) -> u32 {
    let actual: u32 = env
        .storage()
        .instance()
        .get(&DataKey::Contador)
        .unwrap_or(0);
    let nuevo = actual + 1;
    env.storage().instance().set(&DataKey::Contador, &nuevo);
    nuevo
}

#[cfg(test)]
mod test;
