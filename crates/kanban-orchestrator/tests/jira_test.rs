use kanban_orchestrator::jira::JiraClient;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};

#[tokio::test]
async fn search_returns_issues() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "issues": [{
                "key": "AP-1",
                "fields": {
                    "summary": "Hello",
                    "status": { "name": "To Do" },
                    "assignee": null,
                    "comment": { "comments": [] },
                    "description": null,
                    "labels": [],
                    "priority": null,
                    "attachment": []
                }
            }]
        })))
        .mount(&server)
        .await;

    let client = JiraClient::new(server.uri(), "u@x".into(), "tok".into());
    let issues = client.search("project=AP").await.unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].key, "AP-1");
}

#[tokio::test]
async fn transition_resolves_id_then_posts() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/issue/AP-1/transitions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "transitions": [{"id": "31", "name": "Done"}]
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/issue/AP-1/transitions"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let c = JiraClient::new(server.uri(), "e".into(), "t".into());
    c.transition("AP-1", "Done").await.unwrap();
}

fn sample_issue() -> kanban_orchestrator::jira::JiraIssue {
    kanban_orchestrator::jira::JiraIssue {
        key: "AP-1".into(),
        fields: kanban_orchestrator::jira::JiraFields {
            summary: "Test issue".into(),
            status: kanban_orchestrator::jira::JiraNamed {
                name: "To Do".into(),
            },
            assignee: Some(kanban_orchestrator::jira::JiraUser {
                account_id: "user1".into(),
                display_name: "User One".into(),
            }),
            description: None,
            labels: vec![],
            priority: None,
            comment: kanban_orchestrator::jira::JiraComments {
                comments: vec![sample_comment("c1")],
            },
            attachment: vec![],
        },
    }
}

fn sample_comment(id: &str) -> kanban_orchestrator::jira::JiraComment {
    kanban_orchestrator::jira::JiraComment {
        id: id.into(),
        body: serde_json::Value::Null,
        updated: "2026-05-07T00:00:00Z".into(),
    }
}

#[test]
fn diff_detects_new_comment_and_status() {
    use kanban_orchestrator::jira::{JiraDiff, diff_issue};
    let a = sample_issue();
    let mut b = a.clone();
    b.fields.status.name = "Done".into();
    b.fields.comment.comments.push(sample_comment("c2"));
    let d = diff_issue(&a, &b);
    assert!(d.status_changed);
    assert_eq!(d.new_comments.len(), 1);
    assert_eq!(d.new_comments[0].id, "c2");
    assert!(!d.summary_changed);
    assert!(!d.assignee_changed);
}

#[test]
fn diff_detects_terminal_status() {
    use kanban_orchestrator::jira::{TERMINAL_STATUSES, diff_issue};
    let a = sample_issue();
    let mut b = a.clone();
    b.fields.status.name = "Done".into();
    let d = diff_issue(&a, &b);
    assert!(d.status_terminal);
    assert!(TERMINAL_STATUSES.contains(&"Done"));
}
