use serde::Serialize;
use serde::de::DeserializeOwned;
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
    HmacConfig,
    add_headers_to_reqwest,
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
