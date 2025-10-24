use http::Request;
use sha2::{Digest, Sha256};

use gh_broker::model::Priority;
use gh_broker::model::GithubRequest;

fn mk_graphql_body(query: &str, variables_json: &str) -> Vec<u8> {
    let payload = format!(r#"{{"query":{},"variables":{}}}"#, query, variables_json);
    payload.into_bytes()
}

#[test]
fn graphql_requests_with_different_bodies_have_different_keys() {
    let body_a = mk_graphql_body("\"query A\"", "{\"owner\":\"octocat\",\"name\":\"Hello-World\"}");
    let body_b = mk_graphql_body("\"query A\"", "{\"owner\":\"octocat\",\"name\":\"Spoon-Knife\"}");

    let req_a = Request::builder()
        .method("POST")
        .uri("https://api.github.com/graphql")
        .header(http::header::USER_AGENT, "test-agent")
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(body_a)
        .unwrap();
    let req_b = Request::builder()
        .method("POST")
        .uri("https://api.github.com/graphql")
        .header(http::header::USER_AGENT, "test-agent")
        .header(http::header::CONTENT_TYPE, "application/json")
        .body(body_b)
        .unwrap();

    let gh_a = GithubRequest::new(req_a, Priority::Normal).expect("req a");
    let gh_b = GithubRequest::new(req_b, Priority::Normal).expect("req b");

    assert_ne!(gh_a.key(), gh_b.key(), "GraphQL keys must differ for different payloads");
}

#[test]
fn get_requests_ignore_body_in_key() {
    let req_a = Request::builder()
        .method("GET")
        .uri("https://api.github.com/repos/octocat/Hello-World")
        .header(http::header::USER_AGENT, "test-agent")
        .body(Vec::new())
        .unwrap();
    let req_b = Request::builder()
        .method("GET")
        .uri("https://api.github.com/repos/octocat/Hello-World")
        .header(http::header::USER_AGENT, "test-agent")
        .body(b"ignored".to_vec())
        .unwrap();

    let gh_a = GithubRequest::new(req_a, Priority::Normal).expect("req a");
    let gh_b = GithubRequest::new(req_b, Priority::Normal).expect("req b");

    assert_eq!(gh_a.key(), gh_b.key(), "GET keys should not include body");
}

