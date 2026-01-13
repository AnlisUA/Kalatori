use crate::dao::DaoInterface;

use axum::routing::{get, post};
use axum::extract::State;
use rust_decimal::Decimal;

use kalatori_client::utils::HmacConfig;
use kalatori_client::middleware::axum_hmac_validator;
use kalatori_client::types::{CancelInvoiceParams, CreateInvoiceParams, GetInvoiceParams, Invoice, UpdateInvoiceParams};

use crate::types::InvoiceWithIncomingAmount;
use crate::dao::DaoInvoiceError;

use super::ApiState;
use super::utils::{
    ApiResult,
    AppJson,
    AppQuery,
    fallback_handler,
    method_not_allowed_fallback_handler,
};

#[tracing::instrument(skip_all)]
async fn create_invoice<D: DaoInterface>(
    State(state): State<ApiState<D>>,
    AppJson(params): AppJson<CreateInvoiceParams>,
) -> ApiResult<Invoice, DaoInvoiceError> {
    let invoice = state
        .create_invoice(params)
        .await?;

    let with_amount = InvoiceWithIncomingAmount {
        invoice,
        total_received_amount: Decimal::ZERO,
    };

    let result = state.build_public_invoice(with_amount);
    Ok(result.into())
}

#[tracing::instrument(skip_all)]
async fn get_invoice<D: DaoInterface>(
    State(state): State<ApiState<D>>,
    AppQuery(params): AppQuery<GetInvoiceParams>,
) -> ApiResult<Invoice, DaoInvoiceError> {
    let invoice = state
        .get_invoice(params.invoice_id)
        .await?
        .ok_or_else(|| DaoInvoiceError::NotFound {
            invoice_id: params.invoice_id,
        })?;

    let with_amount = InvoiceWithIncomingAmount {
        invoice,
        total_received_amount: Decimal::ZERO,
    };

    let result = state.build_public_invoice(with_amount);
    Ok(result.into())
}

#[tracing::instrument(skip_all)]
async fn update_invoice<D: DaoInterface>(
    State(state): State<ApiState<D>>,
    AppJson(params): AppJson<UpdateInvoiceParams>,
) -> ApiResult<Invoice, DaoInvoiceError> {
    let invoice = state
        .update_invoice(params)
        .await?;

    let with_amount = InvoiceWithIncomingAmount {
        invoice,
        total_received_amount: Decimal::ZERO,
    };

    let result = state.build_public_invoice(with_amount);
    Ok(result.into())
}

#[tracing::instrument(skip_all)]
async fn cancel_invoice<D: DaoInterface>(
    State(state): State<ApiState<D>>,
    AppJson(params): AppJson<CancelInvoiceParams>,
) -> ApiResult<Invoice, DaoInvoiceError> {
    let invoice = state
        .cancel_invoice_admin(params.invoice_id)
        .await?;

    let with_amount = InvoiceWithIncomingAmount {
        invoice,
        total_received_amount: Decimal::ZERO,
    };

    let result = state.build_public_invoice(with_amount);
    Ok(result.into())
}

pub fn routes<D: DaoInterface>(hmac_config: HmacConfig) -> axum::Router<ApiState<D>> {
    axum::Router::new()
        .route("/v3/invoice/create", post(create_invoice))
        .route("/v3/invoice/get", get(get_invoice))
        .route("/v3/invoice/update", post(update_invoice))
        .route("/v3/invoice/cancel", post(cancel_invoice))
        .fallback(fallback_handler)
        .method_not_allowed_fallback(method_not_allowed_fallback_handler)
        .layer(axum::middleware::from_fn_with_state(hmac_config, axum_hmac_validator))
}
