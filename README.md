# Seña de la feria del agricultor

Contrato en Soroban (Stellar) para guardar la seña de un producto hasta que se entrega, se devuelve o se vence el plazo.

Esta entrega es un ejercicio de práctica. No es la propuesta de la hackathon.

## Quién tiene el problema

María quiere un kilo de queso Turrialba en la feria del agricultor. Carlos, el productor, se lo aparta si ella deja una seña. Lo mismo pasa con la miel, los huevos o el café: el productor guarda el producto y deja de ofrecérselo a otra persona.

## Cómo se hace hoy

La seña se deja en efectivo o por SINPE. El comprobante, cuando existe, es un mensaje de WhatsApp. Si María no pasa a recoger, Carlos ya apartó el queso y perdió la venta. Si Carlos se lo vende a otra persona, María no tiene una forma clara de recuperar la plata sin discutir.

## Por qué elegí este problema

Es un problema concreto y pequeño. Hay dos personas, un producto y un solo monto. Pasa en las ferias del agricultor en Costa Rica, y se puede resolver con lo que vimos en clase: direcciones, almacenamiento, `require_auth` y una transferencia de token. No intenta arreglar toda la feria.

## Qué hace el contrato

El comprador deposita la seña en el contrato. El dinero no queda en la cuenta del productor. Sale solo por uno de estos caminos:

1. El productor confirma que entregó el producto y el contrato le paga la seña.
2. El productor devuelve la seña, porque no pudo cumplir.
3. Se vence el plazo y el comprador la reclama. El dinero vuelve a su cuenta.

En el ledger del vencimiento el productor todavía puede confirmar. Un ledger después, ya no: solo el comprador puede reclamar. El productor sí puede devolver la seña incluso después del plazo.

Nadie más puede cerrar esa seña. Cerrarla dos veces falla y no mueve el dinero otra vez.

## Qué no hace

No lleva el producto, no convierte colones a otra moneda y no califica al productor. El monto es el número entero que usa el token (en un asset de Stellar con 7 decimales, `25000000` es 2,5 unidades). El contrato no sabe si eso son colones, XLM u otro token: sirve cualquier token compatible con la interfaz de tokens de Soroban.

El plazo tampoco es una fecha de calendario. Es una cantidad de ledgers a partir del ledger actual. Si un ledger dura 5 segundos, 17280 ledgers son un día.

## Cómo funciona

El código está en `src/lib.rs`.

El contrato se llama `FairDeposit`. Las funciones están en inglés; esta explicación sigue en español.

| Función | Quién firma | Qué pasa |
| --- | --- | --- |
| `reserve` | el comprador | Pasa la seña al contrato y devuelve el número de esa seña. |
| `confirm_delivery` | el productor | Si el plazo sigue vigente, le paga la seña. |
| `refund` | el productor | Devuelve la seña al comprador. |
| `claim_expired` | el comprador | Si el plazo ya pasó, recupera la seña. |
| `get_deposit` | nadie | Lee la seña. No mueve dinero. |

Cada seña guarda comprador, productor, token, monto, ledger de vencimiento, estado y una descripción corta del producto (máximo 64 bytes). El estado es `Held` (apartada), `Delivered` (entregada) o `Refunded` (devuelta).

La firma se exige con `require_auth` antes de mover el token. El estado se guarda como cerrado antes de la transferencia de salida, para no pagar la misma seña dos veces. Al apartar, confirmar, devolver o reclamar, el contrato publica un evento: `reserved`, `delivered`, `refunded` o `claimed`.

Si algo no cuadra, el contrato se detiene con uno de estos errores:

| Código | Error | Cuándo |
| --- | --- | --- |
| 1 | `InvalidAmount` | El monto es cero o negativo. |
| 2 | `SameParty` | Comprador y productor son la misma cuenta. |
| 3 | `InvalidDeadline` | El plazo es cero, o no cabe en el número de ledger. |
| 4 | `InvalidDescription` | La descripción va vacía o pasa de 64 bytes. |
| 5 | `NotFound` | Ese número de seña no está. |
| 6 | `AlreadyClosed` | La seña ya se entregó o ya se devolvió. |
| 7 | `NotProducer` | Quien llama no es el productor de esa seña. |
| 8 | `NotBuyer` | Quien llama no es el comprador de esa seña. |
| 9 | `NotYetExpired` | El comprador reclama antes de tiempo. |
| 10 | `AlreadyExpired` | El productor confirma después del plazo. |

## Cómo probarlo

Hace falta Rust 1.88 o más nuevo. Con [rustup](https://rustup.rs):

```bash
rustup update stable
cargo test
```

`cargo test` corre el contrato dentro del entorno de prueba de Soroban. No hace falta desplegarlo ni tener una cuenta en testnet para ver que la lógica funciona. Las 11 pruebas viven en `src/test.rs`. Usan un token de prueba: el comprador empieza con 10 unidades (7 decimales) y aparta 2,5 por un kilo de queso Turrialba.

| Prueba | Qué demuestra |
| --- | --- |
| `buyer_reserves_cheese_and_producer_confirms_delivery` | La seña sale del comprador, queda en el contrato y, al confirmar, pasa al productor. |
| `producer_can_refund_the_deposit` | El productor puede devolverla completa. |
| `buyer_claims_the_deposit_after_it_expires` | Pasado el plazo, el comprador recupera el dinero. |
| `producer_can_still_be_paid_on_the_expiry_ledger` | En el ledger exacto del vencimiento la entrega sigue siendo válida. |
| `two_deposits_do_not_share_funds` | Dos apartados (queso y miel) tienen números distintos y no comparten el saldo. |
| `buyer_cannot_claim_before_the_deadline` | Reclamar temprano falla y el dinero sigue en el contrato. |
| `producer_cannot_collect_after_expiry` | Confirmar tarde falla y el productor no recibe nada. |
| `a_third_party_cannot_close_the_deposit` | Una tercera cuenta no puede cerrar la seña. |
| `a_deposit_cannot_be_closed_twice` | Después de entregar, confirmar o devolver otra vez falla. |
| `invalid_input_is_rejected_and_moves_no_funds` | Monto, personas, plazo o descripción inválidos no mueven saldo. Consultar una seña que no existe falla. |
| `reserve_fails_without_the_buyers_signature` | Sin la autorización del comprador, `reserve` se detiene. |

No borres `Cargo.lock`. Fija las versiones con las que estas pruebas pasan. En particular, deja `ed25519-dalek` en 2.2.0: el SDK 22 declara `>=2` y una 3.x más nueva no compila con esa versión del SDK.
