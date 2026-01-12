use http::HeaderValue;
use serde::Serialize;
use serde::de::DeserializeOwned;
use hmac::Mac;

use crate::types::{GetInvoiceParams, CreateInvoiceParams, Invoice};
use crate::utils::{
    HmacConfig, TIMESTAMP_HEADER, SIGNATURE_HEADER, hmac_from_request_parts, timestamp_secs
};

pub struct KalatoriClient {
    client: reqwest::Client,
    modify_path: fn(&str) -> String,
    base_url: String,
    config: HmacConfig,
}

pub const CREATE_INVOICE_PATH: &str = "/private/v3/invoice/create";
pub const GET_INVOICE_PATH: &str = "/private/v3/invoice/get";

// A way to restrict HTTP methods in the client
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
    pub fn new(base_url: String, secret_key: impl AsRef<str>) -> Self {
        let config = HmacConfig::new(secret_key.as_ref(), 0);

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

    fn add_headers(
        &self,
        request: &mut reqwest::Request,
    ) {
        let timestamp = timestamp_secs().to_string();

        let signature = hmac_from_request_parts(
            &self.config,
            request.method(),
            request.url().path(),
            request.url().query(),
            request
                .body()
                .map(|b| b.as_bytes())
                .flatten()
                .unwrap_or(&[]),
            &timestamp,
        ).unwrap();

        let encoded_signature = const_hex::encode(signature.finalize().into_bytes());

        let headers = request.headers_mut();

        headers.insert(TIMESTAMP_HEADER, HeaderValue::from_str(&timestamp).unwrap());
        headers.insert(SIGNATURE_HEADER, HeaderValue::from_str(&encoded_signature).unwrap());
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

        self.add_headers(&mut request);

        Ok(request)
    }

    async fn execute_request<T: DeserializeOwned>(
        &self,
        request: reqwest::Request,
    ) -> Result<T, reqwest::Error> {
        self.client
            .execute(request)
            .await?
            .json()
            .await
    }

    pub async fn get_invoice(&self, payload: GetInvoiceParams) -> Result<Invoice, reqwest::Error> {
        let request = self.build_request(
            KalatoriHttpMethod::Get,
            GET_INVOICE_PATH,
            payload,
        )?;

        self.execute_request(request).await
    }

    pub async fn create_invoice(&self, payload: CreateInvoiceParams) -> Result<Invoice, reqwest::Error> {
        let request = self.build_request(
            KalatoriHttpMethod::Post,
            CREATE_INVOICE_PATH,
            payload,
        )?;

        self.execute_request(request).await
    }
}

#[cfg(test)]
mod tests {
    use crate::types::InvoiceCart;
    use rust_decimal::Decimal;
    use uuid::Uuid;

    use super::*;

    #[tokio::test]
    async fn test_create_invoice() {
        let client = KalatoriClient::new("http://localhost:16726".to_string(), "secret");

        let params = CreateInvoiceParams {
            amount: Decimal::ONE_HUNDRED,
            order_id: Uuid::new_v4().to_string(),
            cart: InvoiceCart::empty(),
            redirect_url: "localhost:8000/thank-you-page".to_string(),
        };

        let result = client.create_invoice(params).await.unwrap();
        println!("{:?}", result);
    }
}
