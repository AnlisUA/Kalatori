use http::HeaderValue;
use serde::Serialize;
use serde::de::DeserializeOwned;
use hmac::Mac;
use secrecy::SecretSlice;

use crate::types::{
    ApiResult,
    ApiResultStructured,
    GetInvoiceParams,
    CreateInvoiceParams,
    CancelInvoiceParams,
    UpdateInvoiceParams,
    Invoice,
};
use crate::utils::{
    HmacConfig, SIGNATURE_HEADER, TIMESTAMP_HEADER, add_headers_to_reqwest, hmac_from_request_parts, timestamp_secs
};

pub struct KalatoriClient {
    client: reqwest::Client,
    modify_path: fn(&str) -> String,
    base_url: String,
    config: HmacConfig,
}

pub const CREATE_INVOICE_PATH: &str = "/private/v3/invoice/create";
pub const GET_INVOICE_PATH: &str = "/private/v3/invoice/get";
pub const UPDATE_INVOICE_PATH: &str = "/private/v3/invoice/update";
pub const CANCEL_INVOICE_PATH: &str = "/private/v3/invoice/cancel";

// A way to restrict HTTP methods in the client
#[derive(Debug, Clone, Copy)]
enum KalatoriHttpMethod {
    Get,
    Post,
}

impl From<KalatoriHttpMethod> for http::Method {
    fn from(method: KalatoriHttpMethod) -> Self {
        match method {
            KalatoriHttpMethod::Get => http::Method::GET,
            KalatoriHttpMethod::Post => http::Method::POST,
        }
    }
}

impl KalatoriClient {
    // TODO: add host validation
    pub fn new(base_url: String, secret_key: impl Into<SecretSlice<u8>>) -> Self {
        let config = HmacConfig::new(secret_key, 0);

        Self {
            client: reqwest::Client::new(),
            modify_path: |path| path.to_string(),
            base_url,
            config,
        }
    }

    pub fn with_path_modifier(
        mut self,
        modifier: fn(&str) -> String,
    ) -> Self {
        self.modify_path = modifier;
        self
    }

    fn build_url(&self, path: &str) -> String {
        let modified_path = (self.modify_path)(path);
        format!("{}{}", self.base_url, modified_path)
    }

    fn build_request(
        &self,
        method: KalatoriHttpMethod,
        path: &str,
        payload: impl Serialize,
    ) -> Result<reqwest::Request, reqwest::Error> {
        let url = self.build_url(path);

        let mut request = match method {
            KalatoriHttpMethod::Get => self.client.get(url).query(&payload).build()?,
            KalatoriHttpMethod::Post => self.client.post(url).json(&payload).build()?,
        };

        add_headers_to_reqwest(&self.config, &mut request);

        Ok(request)
    }

    async fn execute_request<T: DeserializeOwned>(
        &self,
        request: reqwest::Request,
    ) -> Result<ApiResult<T>, reqwest::Error> {
        let result = self.client
            .execute(request)
            .await?
            .json::<ApiResultStructured<T>>()
            .await?;

        Ok(result.into())
    }

    pub async fn get_invoice(&self, payload: GetInvoiceParams) -> Result<ApiResult<Invoice>, reqwest::Error> {
        let request = self.build_request(
            KalatoriHttpMethod::Get,
            GET_INVOICE_PATH,
            payload,
        )?;

        self.execute_request(request).await
    }

    pub async fn create_invoice(&self, payload: CreateInvoiceParams) -> Result<ApiResult<Invoice>, reqwest::Error> {
        let request = self.build_request(
            KalatoriHttpMethod::Post,
            CREATE_INVOICE_PATH,
            payload,
        )?;

        self.execute_request(request).await
    }

    pub async fn update_invoice(&self, payload: UpdateInvoiceParams) -> Result<ApiResult<Invoice>, reqwest::Error> {
        let request = self.build_request(
            KalatoriHttpMethod::Post,
            UPDATE_INVOICE_PATH,
            payload,
        )?;

        self.execute_request(request).await
    }

    pub async fn cancel_invoice(&self, payload: CancelInvoiceParams) -> Result<ApiResult<Invoice>, reqwest::Error> {
        let request = self.build_request(
            KalatoriHttpMethod::Post,
            CANCEL_INVOICE_PATH,
            payload,
        )?;

        self.execute_request(request).await
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;
    use uuid::Uuid;

    use crate::types::InvoiceCart;

    use super::*;

    // #[tokio::test]
    // async fn test_create_invoice() {
    //     let client = KalatoriClient::new("http://localhost:8080".to_string(), "secret".as_bytes().to_vec());

    //     let params = CreateInvoiceParams {
    //         amount: Decimal::ONE_HUNDRED,
    //         order_id: Uuid::new_v4().to_string(),
    //         cart: InvoiceCart::empty(),
    //         redirect_url: "localhost:8000/thank-you-page".to_string(),
    //     };

    //     let result = client.create_invoice(params).await.unwrap();
    //     println!("{:?}", result);
    // }

    // #[tokio::test]
    // async fn test_invalid_json() {
    //     let client = KalatoriClient::new("http://localhost:16726".to_string(), "secret");

    //     let params = GetInvoiceParams {
    //         invoice_id: Uuid::new_v4(),
    //         include_transaction: false,
    //     };

    //     let request = client.build_request(
    //         KalatoriHttpMethod::Get,
    //         CREATE_INVOICE_PATH,
    //         params,
    //     ).unwrap();

    //     let result = client.execute_request::<Invoice>(request).await;

    //     println!("Result :{:#?}", result);
    // }
}
