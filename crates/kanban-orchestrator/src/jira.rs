use serde::{Deserialize, Serialize};

#[derive(Clone)]
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

#[derive(Debug, Default, Clone)]
pub struct JiraDiff {
    pub status_changed: bool,
    pub status_terminal: bool,
    pub assignee_changed: bool,
    pub summary_changed: bool,
    pub new_comments: Vec<JiraComment>,
    pub description_changed: bool,
}

pub const TERMINAL_STATUSES: &[&str] = &["Done", "Closed", "Cancelled", "Resolved"];

pub fn diff_issue(prev: &JiraIssue, curr: &JiraIssue) -> JiraDiff {
    let prev_ids: std::collections::HashSet<&str> = prev
        .fields
        .comment
        .comments
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    let new_comments: Vec<JiraComment> = curr
        .fields
        .comment
        .comments
        .iter()
        .filter(|c| !prev_ids.contains(c.id.as_str()))
        .cloned()
        .collect();
    JiraDiff {
        status_changed: prev.fields.status.name != curr.fields.status.name,
        status_terminal: TERMINAL_STATUSES.contains(&curr.fields.status.name.as_str()),
        assignee_changed: prev.fields.assignee.as_ref().map(|u| &u.account_id)
            != curr.fields.assignee.as_ref().map(|u| &u.account_id),
        summary_changed: prev.fields.summary != curr.fields.summary,
        new_comments,
        description_changed: prev.fields.description != curr.fields.description,
    }
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
        // Jira Cloud removed /rest/api/3/search (HTTP 410) in 2025 in favour of
        // the cursor-paginated /rest/api/3/search/jql endpoint.
        // Docs: https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-search/
        //
        // Notes:
        //  - `fields` must be specified explicitly; if omitted only `id` comes back.
        //  - `maxResults` is capped at 100.
        //  - Pagination is via `nextPageToken` cursor; loop until it is None.
        #[derive(Deserialize)]
        struct Resp {
            #[serde(default)]
            issues: Vec<JiraIssue>,
            #[serde(default)]
            #[serde(rename = "nextPageToken")]
            next_page_token: Option<String>,
        }

        let url = format!("{}/rest/api/3/search/jql", self.base);
        let fields = "summary,status,assignee,description,labels,priority,comment,attachment";

        let mut all = Vec::new();
        let mut next_token: Option<String> = None;
        loop {
            let mut body = serde_json::json!({
                "jql": jql,
                "fields": fields.split(',').collect::<Vec<_>>(),
                "maxResults": 100,
            });
            if let Some(tok) = &next_token {
                body["nextPageToken"] = serde_json::Value::String(tok.clone());
            }

            let resp: Resp = self
                .http
                .post(&url)
                .basic_auth(&self.email, Some(&self.token))
                .header("Accept", "application/json")
                .json(&body)
                .send()
                .await
                .map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?
                .error_for_status()
                .map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?
                .json()
                .await
                .map_err(|e| crate::OrchestratorError::Jira(e.to_string()))?;

            all.extend(resp.issues);
            match resp.next_page_token {
                Some(tok) if !tok.is_empty() => next_token = Some(tok),
                _ => break,
            }
        }
        Ok(all)
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
