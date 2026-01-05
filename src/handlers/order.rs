use axum::Json;
use axum::extract::{
    Path,
    State as ExtractState,
};
use axum::http::StatusCode;
use axum::response::{
    IntoResponse,
    Response,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive;
use serde::Deserialize;

use crate::dao::{
    DaoInterface,
    DaoInvoiceError,
};
use crate::error::{
    ForceWithdrawalError,
    OrderError,
};
use crate::legacy_types::{
    AMOUNT,
    CURRENCY,
    InvalidParameter,
    LegacyApiData,
    OrderResponse,
    OrderStatus,
    invoice_to_order_info,
    transaction_to_transaction_info,
};
use crate::server::ApiState;
use crate::state::AppState;
use crate::types::{
    Invoice,
    InvoiceCart,
    InvoiceStatus,
};

use super::types::{
    CreateInvoiceParams,
    UpdateInvoiceParams,
};

const EXISTENTIAL_DEPOSIT: f64 = 0.07;

#[derive(Debug, Deserialize)]
pub struct OrderPayload {
    pub amount: Option<f64>,
    pub currency: Option<String>,
    pub callback: Option<String>,
    pub redirect_url: Option<String>,
}

fn build_order_status(
    invoice: Invoice,
    api_data: &LegacyApiData,
    message: String,
) -> Result<OrderStatus, OrderError> {
    let order_info = invoice_to_order_info(&invoice, &api_data.currencies)?;

    Ok(OrderStatus {
        order: invoice.order_id,
        message,
        recipient: api_data.recipient.clone(),
        server_info: api_data.server_info.clone(),
        order_info,
        payment_page: format!(
            "http://localhost:16726/public/v1?invoice_id={}",
            invoice.id
        ),
        redirect_url: String::new(),
    })
}

async fn create_or_update_invoice<D: DaoInterface + Clone + 'static>(
    state: &AppState<D>,
    api_data: &LegacyApiData,
    order_id: String,
    amount: Decimal,
    asset_id: String,
    callback_url: Option<String>,
    redirect_url: Option<String>,
) -> Result<OrderResponse, OrderError> {
    // We actually perform duplicated work here, as we first check for existing
    // invoice and in case of update we again fetch the invoice by invoice_id.
    // It is made in order to keep backward compatibility but not complicate the
    // new interface with "get by order_id" functionality.
    if let Some(invoice) = state
        .get_invoice_by_order_id(&order_id)
        .await
        .map_err(|_| OrderError::InternalError)?
    {
        if invoice.status != InvoiceStatus::Waiting {
            let order_status = build_order_status(invoice, api_data, String::new())?;
            return Ok(OrderResponse::CollidedOrder(
                order_status,
            ));
        }

        let params = UpdateInvoiceParams {
            invoice_id: invoice.id,
            amount: Some(amount),
            cart: None,
        };

        let updated = state
            .update_invoice(params)
            .await
            .map_err(|_| OrderError::InternalError)?;

        let order_status = build_order_status(updated, api_data, String::new())?;

        Ok(OrderResponse::ModifiedOrder(
            order_status,
        ))
    } else {
        let params = CreateInvoiceParams {
            order_id,
            // TODO: better return an error if parsing fails
            amount,
            asset_id: Some(asset_id),
            cart: InvoiceCart::empty(),
            callback_url,
            redirect_url: redirect_url.unwrap_or_default(),
        };

        let invoice = state
            .create_invoice(params)
            .await
            .map_err(|_| OrderError::InternalError)?;

        let order_status = build_order_status(invoice, api_data, String::new())?;
        Ok(OrderResponse::NewOrder(order_status))
    }
}

async fn process_order<D: DaoInterface + Clone + 'static>(
    state: &AppState<D>,
    order_id: String,
    order_payload: Option<OrderPayload>,
    api_data: &LegacyApiData,
) -> Result<OrderResponse, OrderError> {
    if let Some(payload) = order_payload {
        // AMOUNT validation
        let Some(amount) = payload.amount else {
            return Err(OrderError::MissingParameter(
                AMOUNT.to_string(),
            ));
        };

        if amount < EXISTENTIAL_DEPOSIT {
            return Err(OrderError::LessThanExistentialDeposit(
                EXISTENTIAL_DEPOSIT,
            ));
        }

        // CURRENCY validation
        let Some(currency) = payload.currency else {
            return Err(OrderError::MissingParameter(
                CURRENCY.to_string(),
            ));
        };

        api_data
            .currencies
            .get(&currency)
            .ok_or(OrderError::UnknownCurrency)?;

        let asset_id = match currency.to_lowercase().as_str() {
            "usdc" => 1337,
            "usdt" => 1984,
            _ => return Err(OrderError::UnknownCurrency),
        }
        .to_string();

        let amount = Decimal::from_f64(amount).unwrap();

        create_or_update_invoice(
            state,
            api_data,
            order_id,
            amount,
            asset_id,
            payload.callback,
            payload.redirect_url,
        )
        .await
    } else {
        let invoice = state
            .get_invoice_by_order_id(&order_id)
            .await
            .map_err(|_| OrderError::InternalError)?;

        match invoice {
            Some(invoice) => {
                let transactions = state
                    .get_invoice_transactions(invoice.id)
                    .await
                    .map_err(|_| OrderError::InternalError)?
                    .into_iter()
                    .map(|tx| transaction_to_transaction_info(tx, &api_data.currencies))
                    .collect::<Result<Vec<_>, _>>()?;

                let mut order_status = build_order_status(invoice, api_data, String::new())?;
                order_status.order_info.transactions = transactions;
                Ok(OrderResponse::FoundOrder(order_status))
            },
            None => Ok(OrderResponse::NotFound),
        }
    }
}

pub async fn order<D: DaoInterface + Clone + 'static>(
    ExtractState(state): ExtractState<ApiState<D>>,
    Path(order_id): Path<String>,
    payload: Option<Json<OrderPayload>>,
) -> Response {
    let data = payload.map(|p| p.0);

    match process_order(&state.state, order_id, data, &state.legacy_api_data).await {
        Ok(order) => match order {
            OrderResponse::NewOrder(order_status) => (StatusCode::CREATED, Json(order_status)).into_response(),
            // TODO: behaviour is exactly the same for the quite different cases.
            // Perhaps need to identify what exactly happened by additional flag or status code?
            OrderResponse::FoundOrder(order_status) |
            OrderResponse::ModifiedOrder(order_status) => (StatusCode::OK, Json(order_status)).into_response(),
            OrderResponse::CollidedOrder(order_status) => (StatusCode::CONFLICT, Json(order_status)).into_response(),
            OrderResponse::NotFound => (StatusCode::NOT_FOUND, "").into_response(),
        },
        Err(error) => match error {
            OrderError::LessThanExistentialDeposit(existential_deposit) => (
                StatusCode::BAD_REQUEST,
                Json([InvalidParameter {
                    parameter: AMOUNT.into(),
                    message: format!("provided amount is less than the currency's existential deposit ({existential_deposit})"),
                }]),
            )
                .into_response(),
            OrderError::UnknownCurrency => (
                StatusCode::BAD_REQUEST,
                Json([InvalidParameter {
                    parameter: CURRENCY.into(),
                    message: "provided currency isn't supported".into(),
                }]),
            )
                .into_response(),
            OrderError::MissingParameter(parameter) => (
                StatusCode::BAD_REQUEST,
                Json([InvalidParameter {
                    parameter,
                    message: "parameter wasn't found".into(),
                }]),
            )
                .into_response(),
            OrderError::InvalidParameter(parameter) => (
                StatusCode::BAD_REQUEST,
                Json([InvalidParameter {
                    parameter,
                    message: "parameter's format is invalid".into(),
                }]),
            )
                .into_response(),
            OrderError::InternalError => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
    }
}

pub async fn process_force_withdrawal<D: DaoInterface + Clone + 'static>(
    state: &ApiState<D>,
    order_id: String,
) -> Result<OrderResponse, ForceWithdrawalError> {
    match state
        .state
        .force_withdrawal(order_id)
        .await
    {
        Ok(invoice) => {
            let order_status = build_order_status(
                invoice,
                &state.legacy_api_data,
                "Force withdrawal initiated".to_string(),
            )
            .map_err(|_| ForceWithdrawalError::WithdrawalError(String::new()))?;

            Ok(OrderResponse::FoundOrder(order_status))
        },
        Err(DaoInvoiceError::NotFound {
            ..
        }) => Ok(OrderResponse::NotFound),
        Err(other) => Err(ForceWithdrawalError::WithdrawalError(
            other.to_string(),
        )),
    }
}

pub async fn force_withdrawal<D: DaoInterface + Clone + 'static>(
    ExtractState(state): ExtractState<ApiState<D>>,
    Path(order_id): Path<String>,
) -> Response {
    match process_force_withdrawal(&state, order_id).await {
        Ok(OrderResponse::FoundOrder(order_status)) => {
            (StatusCode::CREATED, Json(order_status)).into_response()
        },
        Ok(OrderResponse::NotFound) => (StatusCode::NOT_FOUND, "Order not found").into_response(),
        Err(ForceWithdrawalError::WithdrawalError(a)) => {
            (StatusCode::BAD_REQUEST, Json(a)).into_response()
        },
        Err(ForceWithdrawalError::MissingParameter(parameter)) => (
            StatusCode::BAD_REQUEST,
            Json([InvalidParameter {
                parameter,
                message: "parameter wasn't found".into(),
            }]),
        )
            .into_response(),
        Err(ForceWithdrawalError::InvalidParameter(parameter)) => (
            StatusCode::BAD_REQUEST,
            Json([InvalidParameter {
                parameter,
                message: "parameter's format is invalid".into(),
            }]),
        )
            .into_response(),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unexpected response type for force withdrawal",
        )
            .into_response(),
    }
}
