use kanban_orchestrator::jira::{
    JiraComment, JiraComments, JiraFields, JiraIssue, JiraNamed, diff_issue,
};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};

#[tokio::test]
async fn search_returns_issues() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/search/jql"))
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
            }],
            "isLast": true
        })))
        .mount(&server)
        .await;

    let client =
        kanban_orchestrator::jira::JiraClient::new(server.uri(), "u@x".into(), "tok".into());
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
    let c = kanban_orchestrator::jira::JiraClient::new(server.uri(), "e".into(), "t".into());
    c.transition("AP-1", "Done").await.unwrap();
}

fn sample_issue() -> JiraIssue {
    JiraIssue {
        key: "AP-1".into(),
        fields: JiraFields {
            summary: "Test".into(),
            status: JiraNamed {
                name: "To Do".into(),
            },
            assignee: None,
            description: None,
            labels: vec![],
            priority: None,
            comment: JiraComments { comments: vec![] },
            attachment: vec![],
        },
    }
}

fn sample_comment(id: &str) -> JiraComment {
    JiraComment {
        id: id.into(),
        body: serde_json::Value::Null,
        updated: "2026-05-07T00:00:00Z".into(),
    }
}

#[test]
fn diff_detects_new_comment_and_status() {
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
