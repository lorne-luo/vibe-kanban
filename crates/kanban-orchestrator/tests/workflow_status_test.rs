use std::path::{Path, PathBuf};

use db::models::repo::Repo;
use kanban_orchestrator::workflow_status::{
    ProjectWorkflowState, RepoWorkflowState, RepoWorkflowStatus, WorkflowEntryState,
    aggregate_project_state, compute_for_repo,
};

fn make_repo(path: PathBuf, name: &str) -> Repo {
    Repo {
        id: uuid::Uuid::new_v4(),
        path,
        name: name.to_string(),
        display_name: name.to_string(),
        setup_script: None,
        cleanup_script: None,
        archive_script: None,
        copy_files: None,
        parallel_setup_script: false,
        dev_server_script: None,
        default_target_branch: None,
        default_working_dir: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

fn write_agents(dir: &Path, names: &[&str]) {
    std::fs::create_dir_all(dir.join(".agents/agent")).unwrap();
    for n in names {
        std::fs::write(
            dir.join(format!(".agents/agent/{}.md", n)),
            "---\nname: x\n---\nbody",
        )
        .unwrap();
    }
}

fn write_workflow(dir: &Path, name: &str, fixture: &str) {
    std::fs::create_dir_all(dir.join(".agents/kanban-workflows")).unwrap();
    std::fs::copy(
        format!("tests/fixtures/{fixture}"),
        dir.join(format!(".agents/kanban-workflows/{name}.yml")),
    )
    .unwrap();
}

fn dummy_repo_status(state: RepoWorkflowState) -> RepoWorkflowStatus {
    RepoWorkflowStatus {
        repo_id: uuid::Uuid::nil(),
        repo_name: "r".into(),
        workflows_dir: "/tmp".into(),
        state,
        workflows: Vec::new(),
        error: None,
    }
}

#[tokio::test]
async fn repo_without_agents_dir_is_not_configured() {
    let dir = tempfile::tempdir().unwrap();
    let repo = make_repo(dir.path().to_path_buf(), "r1");
    let status = compute_for_repo(&repo).await;
    assert!(matches!(status.state, RepoWorkflowState::NotConfigured));
    assert!(status.workflows.is_empty());
}

#[tokio::test]
async fn repo_with_workflows_dir_but_no_yml_is_not_configured() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".agents/kanban-workflows")).unwrap();
    let repo = make_repo(dir.path().to_path_buf(), "r1");
    let status = compute_for_repo(&repo).await;
    assert!(matches!(status.state, RepoWorkflowState::NotConfigured));
    assert!(status.workflows.is_empty());
}

#[tokio::test]
async fn nonexistent_repo_path_is_invalid() {
    let repo = make_repo(PathBuf::from("/nonexistent/__definitely_not_a_dir__"), "r1");
    let status = compute_for_repo(&repo).await;
    assert!(matches!(status.state, RepoWorkflowState::Invalid));
    assert!(
        status
            .error
            .as_deref()
            .unwrap()
            .contains("repo path inaccessible")
    );
}

#[tokio::test]
async fn valid_workflow_is_ready() {
    let dir = tempfile::tempdir().unwrap();
    write_agents(dir.path(), &["analyzer", "coder", "reviewer"]);
    write_workflow(dir.path(), "AP", "valid_workflow.yml");
    let repo = make_repo(dir.path().to_path_buf(), "r1");
    let status = compute_for_repo(&repo).await;
    assert!(
        matches!(status.state, RepoWorkflowState::Ready),
        "state: {:?}, workflows: {:?}",
        status.state,
        status.workflows
    );
    assert_eq!(status.workflows.len(), 1);
    assert!(matches!(status.workflows[0].state, WorkflowEntryState::Ok));
}

#[tokio::test]
async fn yml_with_missing_agent_file_is_missing_agent() {
    let dir = tempfile::tempdir().unwrap();
    // only analyzer.md; valid_workflow.yml references coder + reviewer too
    write_agents(dir.path(), &["analyzer"]);
    write_workflow(dir.path(), "AP", "valid_workflow.yml");
    let repo = make_repo(dir.path().to_path_buf(), "r1");
    let status = compute_for_repo(&repo).await;
    assert!(matches!(status.state, RepoWorkflowState::Invalid));
    assert_eq!(status.workflows.len(), 1);
    assert!(matches!(
        status.workflows[0].state,
        WorkflowEntryState::MissingAgentFile
    ));
}

#[tokio::test]
async fn yml_with_dangling_next_is_invalid_yaml() {
    let dir = tempfile::tempdir().unwrap();
    write_agents(dir.path(), &["analyzer", "coder", "reviewer"]);
    write_workflow(dir.path(), "BROKEN", "dangling_next.yml");
    let repo = make_repo(dir.path().to_path_buf(), "r1");
    let status = compute_for_repo(&repo).await;
    assert!(matches!(status.state, RepoWorkflowState::Invalid));
    assert!(matches!(
        status.workflows[0].state,
        WorkflowEntryState::InvalidYaml
    ));
}

#[test]
fn aggregate_invalid_wins() {
    let repos = vec![
        dummy_repo_status(RepoWorkflowState::Ready),
        dummy_repo_status(RepoWorkflowState::Invalid),
    ];
    assert!(matches!(
        aggregate_project_state(&repos),
        ProjectWorkflowState::Invalid
    ));
}

#[test]
fn aggregate_ready_when_any_ready_and_no_invalid() {
    let repos = vec![
        dummy_repo_status(RepoWorkflowState::Ready),
        dummy_repo_status(RepoWorkflowState::NotConfigured),
    ];
    assert!(matches!(
        aggregate_project_state(&repos),
        ProjectWorkflowState::Ready
    ));
}

#[test]
fn aggregate_all_not_configured() {
    let repos = vec![
        dummy_repo_status(RepoWorkflowState::NotConfigured),
        dummy_repo_status(RepoWorkflowState::NotConfigured),
    ];
    assert!(matches!(
        aggregate_project_state(&repos),
        ProjectWorkflowState::NotConfigured
    ));
}
