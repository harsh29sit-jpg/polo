use chrono::{DateTime, Utc};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};

use polo_core::{
    branch::BranchInfo,
    fact::{BranchName, Fact, FactId, Namespace, TxId, Value},
    merge::{DiffEntry, MergeResult},
    namespace::NamespaceInfo,
    tx::Transaction,
    Hlc,
};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("HTTP error {status}: {body}")]
    Api { status: u16, body: String },

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("parse error: {0}")]
    Parse(String),
}

pub struct PoloClient {
    base: String,
    client: Client,
    ns: String,
    branch: String,
    author: Option<String>,
}

impl PoloClient {
    pub fn new(addr: impl Into<String>) -> Self {
        let base = addr.into().trim_end_matches('/').to_string();
        Self {
            base,
            client: Client::new(),
            ns: "default".into(),
            branch: "main".into(),
            author: None,
        }
    }

    pub fn with_token(self, token: impl Into<String>) -> Self {
        let token = token.into();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        let client = Client::builder()
            .default_headers(headers)
            .build()
            .expect("failed to build HTTP client");
        Self { client, ..self }
    }

    pub fn namespace(mut self, ns: impl Into<String>) -> Self {
        self.ns = ns.into();
        self
    }

    pub fn branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = branch.into();
        self
    }

    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }

    async fn check_response(&self, resp: reqwest::Response) -> Result<reqwest::Response, ClientError> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        Err(ClientError::Api {
            status: status.as_u16(),
            body,
        })
    }

    pub async fn ping(&self) -> Result<(), ClientError> {
        let resp = self.client.get(format!("{}/healthz", self.base)).send().await?;
        self.check_response(resp).await?;
        Ok(())
    }

    pub async fn record(
        &self,
        entity: &str,
        attr: &str,
        value: Value,
        opts: RecordOpts,
    ) -> Result<RecordResult, ClientError> {
        #[derive(Serialize)]
        struct Body<'a> {
            entity: &'a str,
            attr: &'a str,
            value: &'a Value,
            branch: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            author: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            message: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            valid_from: Option<DateTime<Utc>>,
            #[serde(skip_serializing_if = "Option::is_none")]
            valid_to: Option<DateTime<Utc>>,
            #[serde(skip_serializing_if = "Option::is_none")]
            caused_by: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            idempotency_key: Option<String>,
        }

        let body = Body {
            entity,
            attr,
            value: &value,
            branch: opts.branch.as_deref().unwrap_or(&self.branch),
            author: opts.author.as_deref().or(self.author.as_deref()),
            message: opts.message.as_deref(),
            valid_from: opts.valid_from,
            valid_to: opts.valid_to,
            caused_by: opts.caused_by.map(|t| t.to_string()),
            idempotency_key: opts.idempotency_key,
        };

        let resp = self
            .client
            .post(format!("{}/v1/{}/facts", self.base, self.ns))
            .json(&body)
            .send()
            .await?;

        self.check_response(resp)
            .await?
            .json::<RecordResult>()
            .await
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    pub async fn asof(
        &self,
        entity: &str,
        attr: &str,
        at: Option<Hlc>,
        branch: Option<&str>,
    ) -> Result<Option<Fact>, ClientError> {
        let mut url = format!(
            "{}/v1/{}/asof?entity={}&attr={}",
            self.base, self.ns, entity, attr
        );
        if let Some(hlc) = at {
            url.push_str(&format!("&at={hlc}"));
        }
        if let Some(br) = branch {
            url.push_str(&format!("&branch={br}"));
        } else {
            url.push_str(&format!("&branch={}", self.branch));
        }

        let resp = self.client.get(&url).send().await?;
        let resp = self.check_response(resp).await?;
        resp.json::<Option<Fact>>()
            .await
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    pub async fn effective(
        &self,
        entity: &str,
        attr: &str,
        at: Option<DateTime<Utc>>,
        branch: Option<&str>,
    ) -> Result<Option<Fact>, ClientError> {
        let at_str = at
            .map(|t| t.to_rfc3339())
            .unwrap_or_else(|| Utc::now().to_rfc3339());
        let br = branch.unwrap_or(&self.branch);
        let url = format!(
            "{}/v1/{}/effective?entity={}&attr={}&at={}&branch={}",
            self.base, self.ns, entity, attr, at_str, br
        );

        let resp = self.client.get(&url).send().await?;
        let resp = self.check_response(resp).await?;
        resp.json::<Option<Fact>>()
            .await
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    pub async fn history(
        &self,
        entity: &str,
        attr: &str,
        branch: Option<&str>,
    ) -> Result<Vec<Fact>, ClientError> {
        let br = branch.unwrap_or(&self.branch);
        let url = format!(
            "{}/v1/{}/history?entity={}&attr={}&branch={}",
            self.base, self.ns, entity, attr, br
        );
        let resp = self.client.get(&url).send().await?;
        self.check_response(resp)
            .await?
            .json::<Vec<Fact>>()
            .await
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    pub async fn snapshot(
        &self,
        entity: &str,
        branch: Option<&str>,
    ) -> Result<Vec<Fact>, ClientError> {
        let br = branch.unwrap_or(&self.branch);
        let url = format!(
            "{}/v1/{}/snapshot/{}?branch={}",
            self.base, self.ns, entity, br
        );
        let resp = self.client.get(&url).send().await?;
        self.check_response(resp)
            .await?
            .json::<Vec<Fact>>()
            .await
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    pub async fn retract(
        &self,
        fact_id: &FactId,
        branch: Option<&str>,
        author: Option<&str>,
    ) -> Result<TxId, ClientError> {
        let br = branch.unwrap_or(&self.branch);
        let auth = author.or(self.author.as_deref());
        let mut url = format!(
            "{}/v1/{}/facts/{}?branch={}",
            self.base, self.ns, fact_id, br
        );
        if let Some(a) = auth {
            url.push_str(&format!("&author={a}"));
        }
        let resp = self.client.delete(&url).send().await?;
        #[derive(Deserialize)]
        struct R { tx_id: String }
        let r = self
            .check_response(resp)
            .await?
            .json::<R>()
            .await
            .map_err(|e| ClientError::Parse(e.to_string()))?;
        r.tx_id.parse::<TxId>().map_err(|e| ClientError::Parse(e.to_string()))
    }

    pub async fn list_branches(&self) -> Result<Vec<BranchInfo>, ClientError> {
        let url = format!("{}/v1/{}/branches", self.base, self.ns);
        let resp = self.client.get(&url).send().await?;
        self.check_response(resp)
            .await?
            .json()
            .await
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    pub async fn create_branch(
        &self,
        name: &str,
        from: Option<&str>,
    ) -> Result<(), ClientError> {
        #[derive(Serialize)]
        struct Body<'a> {
            name: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            from: Option<&'a str>,
        }
        let url = format!("{}/v1/{}/branches", self.base, self.ns);
        let resp = self
            .client
            .post(&url)
            .json(&Body { name, from })
            .send()
            .await?;
        self.check_response(resp).await?;
        Ok(())
    }

    pub async fn delete_branch(&self, name: &str) -> Result<(), ClientError> {
        let url = format!("{}/v1/{}/branches/{}", self.base, self.ns, name);
        let resp = self.client.delete(&url).send().await?;
        self.check_response(resp).await?;
        Ok(())
    }

    pub async fn merge(
        &self,
        source: &str,
        target: &str,
        author: Option<&str>,
        message: Option<&str>,
    ) -> Result<MergeResult, ClientError> {
        #[derive(Serialize)]
        struct Body<'a> {
            source: &'a str,
            target: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            author: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            message: Option<&'a str>,
        }
        let url = format!("{}/v1/{}/merge", self.base, self.ns);
        let resp = self
            .client
            .post(&url)
            .json(&Body { source, target, author, message })
            .send()
            .await?;
        self.check_response(resp)
            .await?
            .json()
            .await
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    pub async fn diff(
        &self,
        source: &str,
        target: Option<&str>,
    ) -> Result<Vec<DiffEntry>, ClientError> {
        let tgt = target.unwrap_or("main");
        let url = format!(
            "{}/v1/{}/diff?source={}&target={}",
            self.base, self.ns, source, tgt
        );
        let resp = self.client.get(&url).send().await?;
        self.check_response(resp)
            .await?
            .json()
            .await
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    pub async fn query(&self, pql: &str, branch: Option<&str>) -> Result<Vec<serde_json::Value>, ClientError> {
        #[derive(Serialize)]
        struct Body<'a> {
            pql: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            branch: Option<&'a str>,
        }
        #[derive(Deserialize)]
        struct Resp { rows: Vec<serde_json::Value> }

        let url = format!("{}/v1/{}/query", self.base, self.ns);
        let resp = self
            .client
            .post(&url)
            .json(&Body { pql, branch })
            .send()
            .await?;
        let r = self
            .check_response(resp)
            .await?
            .json::<Resp>()
            .await
            .map_err(|e| ClientError::Parse(e.to_string()))?;
        Ok(r.rows)
    }

    pub async fn list_namespaces(&self) -> Result<Vec<NamespaceInfo>, ClientError> {
        let url = format!("{}/namespaces", self.base);
        let resp = self.client.get(&url).send().await?;
        self.check_response(resp)
            .await?
            .json()
            .await
            .map_err(|e| ClientError::Parse(e.to_string()))
    }

    pub async fn create_namespace(
        &self,
        name: &str,
        merge_policy: Option<&str>,
    ) -> Result<(), ClientError> {
        #[derive(Serialize)]
        struct Body<'a> {
            name: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            merge_policy: Option<&'a str>,
        }
        let url = format!("{}/namespaces", self.base);
        let resp = self
            .client
            .post(&url)
            .json(&Body { name, merge_policy })
            .send()
            .await?;
        self.check_response(resp).await?;
        Ok(())
    }

    pub async fn list_tx(&self, branch: Option<&str>, limit: Option<usize>) -> Result<Vec<Transaction>, ClientError> {
        let br = branch.unwrap_or(&self.branch);
        let lim = limit.unwrap_or(50);
        let url = format!(
            "{}/v1/{}/transactions?branch={}&limit={}",
            self.base, self.ns, br, lim
        );
        let resp = self.client.get(&url).send().await?;
        self.check_response(resp)
            .await?
            .json()
            .await
            .map_err(|e| ClientError::Parse(e.to_string()))
    }
}

#[derive(Debug, Default)]
pub struct RecordOpts {
    pub branch: Option<String>,
    pub author: Option<String>,
    pub message: Option<String>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_to: Option<DateTime<Utc>>,
    pub caused_by: Option<TxId>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RecordResult {
    pub fact_id: String,
    pub tx_id: String,
    pub was_duplicate: bool,
}
