use kanban_orchestrator::jira::JiraClient;
use wiremock::{matchers::*, Mock, MockServer, ResponseTemplate};

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
