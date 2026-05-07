use serde::{Deserialize, Serialize};

pub struct JiraClient {
    base: String,
    email: String,
    token: String,
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JiraIssue {
    pub key: String,
    pub fields: JiraFields,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JiraFields {
    pub summary: String,
    pub status: JiraNamed,
    #[serde(default)]
    pub assignee: Option<JiraUser>,
    #[serde(default)]
    pub description: Option<serde_json::Value>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub priority: Option<JiraNamed>,
    #[serde(default)]
    pub comment: JiraComments,
    #[serde(default)]
    pub attachment: Vec<JiraAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JiraNamed {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JiraUser {
    #[serde(rename = "accountId")]
    pub account_id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct JiraComments {
    #[serde(default)]
    pub comments: Vec<JiraComment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JiraComment {
    pub id: String,
    pub body: serde_json::Value,
    #[serde(rename = "updated")]
    pub updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JiraAttachment {
    pub id: String,
    pub filename: String,
}

impl JiraClient {
    pub fn new(base: String, email: String, token: String) -> Self {
        Self {
            base,
            email,
            token,
            http: reqwest::Client::new(),
        }
    }

    pub async fn search(&self, jql: &str) -> crate::Result<Vec<JiraIssue>> {
        #[derive(Deserialize)]
        struct Resp {
            issues: Vec<JiraIssue>,
        }
        let url = format!("{}/rest/api/3/search", self.base);
        let resp: Resp = self
            .http
            .get(&url)
            .basic_auth(&self.email, Some(&self.token))
            .query(&[("jql", jql), ("maxResults", "200")])
            .send()
            .await
            .map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?
            .error_for_status()
            .map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?
            .json()
            .await
            .map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?;
        Ok(resp.issues)
    }

    pub async fn transition(&self, key: &str, transition_name: &str) -> crate::Result<()> {
        let list_url = format!("{}/rest/api/3/issue/{}/transitions", self.base, key);
        #[derive(Deserialize)]
        struct TList {
            transitions: Vec<TItem>,
        }
        #[derive(Deserialize)]
        struct TItem {
            id: String,
            name: String,
        }
        let list: TList = self
            .http
            .get(&list_url)
            .basic_auth(&self.email, Some(&self.token))
            .send()
            .await
            .map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?
            .error_for_status()
            .map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?
            .json()
            .await
            .map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?;
        let id = list
            .transitions
            .iter()
            .find(|t| t.name == transition_name)
            .ok_or_else(|| {
                crate::OrchestratorError::Jira(format!(
                    "transition '{}' not found",
                    transition_name
                ))
            })?
            .id
            .clone();
        self.http
            .post(&list_url)
            .basic_auth(&self.email, Some(&self.token))
            .json(&serde_json::json!({"transition": {"id": id}}))
            .send()
            .await
            .map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?
            .error_for_status()
            .map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?;
        Ok(())
    }
}
