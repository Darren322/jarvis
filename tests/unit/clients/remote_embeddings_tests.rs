use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

use super::{
    DIMENSIONS, EmbeddingItem, EmbeddingResponse, MAX_INPUT_TOKENS, MAX_RESPONSE_BYTES,
    QUERY_PREFIX, RemoteEmbeddings, RemoteEmbeddingsError, unit_vector,
};
use crate::clients::local_endpoint::LocalEndpointError;
use crate::config::EmbeddingConfig;

fn vector(first: f32, dimensions: usize) -> Vec<f32> {
    let mut vector = vec![0.0; dimensions];
    vector[0] = first;
    vector
}

fn embedding_body(vectors: &[Vec<f32>]) -> Value {
    json!({
        "object": "list",
        "data": vectors
            .iter()
            .enumerate()
            .map(|(index, embedding)| json!({"object": "embedding", "index": index, "embedding": embedding}))
            .collect::<Vec<_>>(),
    })
}

fn client(server: &MockServer) -> RemoteEmbeddings {
    RemoteEmbeddings::new(&EmbeddingConfig {
        base_url: format!("{}/v1", server.uri()),
        model: "bge-test".to_owned(),
    })
    .expect("a localhost URL is allowed")
}

async fn server_returning_unit_vectors() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(embedding_body(&[vector(1.0, DIMENSIONS)])),
        )
        .mount(&server)
        .await;
    server
}

/// (path, JSON body) of every request, in arrival order.
async fn requests(server: &MockServer) -> Vec<(String, Value)> {
    server
        .received_requests()
        .await
        .expect("request recording is enabled")
        .iter()
        .map(|request| {
            let body = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            (request.url.path().to_owned(), body)
        })
        .collect()
}

/// Every case sits on one side of a validation edge.
#[test]
fn only_one_finite_384_dimension_unit_vector_is_accepted() {
    let item = |embedding| EmbeddingItem { embedding };
    let cases: Vec<(&str, Vec<EmbeddingItem>, bool)> = vec![
        ("one unit vector", vec![item(vector(1.0, 384))], true),
        ("no vectors", vec![], false),
        (
            "two vectors",
            vec![item(vector(1.0, 384)), item(vector(1.0, 384))],
            false,
        ),
        ("383 dimensions", vec![item(vector(1.0, 383))], false),
        ("385 dimensions", vec![item(vector(1.0, 385))], false),
        ("norm 1.0009", vec![item(vector(1.0009, 384))], true),
        ("norm 1.0011", vec![item(vector(1.0011, 384))], false),
        ("norm 0.9991", vec![item(vector(0.9991, 384))], true),
        ("norm 0.9989", vec![item(vector(0.9989, 384))], false),
        ("NaN", vec![item(vector(f32::NAN, 384))], false),
        ("infinity", vec![item(vector(f32::INFINITY, 384))], false),
    ];
    for (label, data, accepted) in cases {
        let result = unit_vector(EmbeddingResponse { data });
        assert_eq!(result.is_ok(), accepted, "{label}: {result:?}");
    }
}

#[tokio::test]
async fn a_query_is_prefixed_once_and_a_document_is_sent_raw() {
    let server = server_returning_unit_vectors().await;
    let embeddings = client(&server);

    embeddings
        .embed_query("Where are my keys?")
        .await
        .expect("query embeds");
    embeddings
        .embed_document("The keys are in the bowl.")
        .await
        .expect("document embeds");

    let inputs = requests(&server)
        .await
        .into_iter()
        .map(|(path, body)| {
            assert_eq!(path, "/v1/embeddings");
            assert_eq!(body["model"], "bge-test");
            body["input"]
                .as_str()
                .expect("short input is text")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        inputs,
        vec![
            format!("{QUERY_PREFIX}Where are my keys?"),
            "The keys are in the bowl.".to_owned(),
        ]
    );
}

#[tokio::test]
async fn only_text_that_might_exceed_the_token_window_is_tokenized_first() {
    for (bytes, tokenized) in [(MAX_INPUT_TOKENS - 2, false), (MAX_INPUT_TOKENS - 1, true)] {
        let server = server_returning_unit_vectors().await;
        Mock::given(method("POST"))
            .and(path("/tokenize"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"tokens": [101, 7, 102]})),
            )
            .mount(&server)
            .await;

        client(&server)
            .embed_document(&"a".repeat(bytes))
            .await
            .expect("document embeds");

        let paths = requests(&server)
            .await
            .into_iter()
            .map(|(path, _)| path)
            .collect::<Vec<_>>();
        let expected = if tokenized {
            vec!["/tokenize", "/v1/embeddings"]
        } else {
            vec!["/v1/embeddings"]
        };
        assert_eq!(paths, expected, "{bytes} bytes");
    }
}

#[tokio::test]
async fn input_over_512_tokens_keeps_the_first_511_and_the_closing_token() {
    for token_count in [MAX_INPUT_TOKENS, MAX_INPUT_TOKENS + 1] {
        let server = server_returning_unit_vectors().await;
        // [CLS], then body tokens, then [SEP], as llama.cpp returns them.
        let body_tokens = (1000..).take(token_count - 2);
        let tokens = std::iter::once(101)
            .chain(body_tokens)
            .chain(std::iter::once(102))
            .collect::<Vec<u32>>();
        Mock::given(method("POST"))
            .and(path("/tokenize"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"tokens": tokens})))
            .mount(&server)
            .await;

        client(&server)
            .embed_document(&"word ".repeat(200))
            .await
            .expect("document embeds");

        let requests = requests(&server).await;
        assert_eq!(requests[0].1["add_special"], true);
        let sent = serde_json::from_value::<Vec<u32>>(requests[1].1["input"].clone())
            .expect("long input is sent as token IDs");
        assert_eq!(sent.len(), MAX_INPUT_TOKENS, "{token_count} tokens");
        assert_eq!(sent[..MAX_INPUT_TOKENS - 1], tokens[..MAX_INPUT_TOKENS - 1]);
        assert_eq!(sent.last(), Some(&102), "{token_count} tokens");
    }
}

#[tokio::test]
async fn a_response_over_64_kib_is_rejected_without_being_parsed() {
    for (size, accepted) in [(MAX_RESPONSE_BYTES, true), (MAX_RESPONSE_BYTES + 1, false)] {
        let server = MockServer::start().await;
        // Trailing whitespace keeps the JSON valid at any size.
        let mut body = embedding_body(&[vector(1.0, DIMENSIONS)]).to_string();
        body.push_str(&" ".repeat(size - body.len()));
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/json"))
            .mount(&server)
            .await;

        let result = client(&server).embed_document("hello").await;
        match (accepted, result) {
            (true, Ok(_)) | (false, Err(RemoteEmbeddingsError::TooLarge)) => {}
            (_, other) => panic!("{size} bytes: {other:?}"),
        }
    }
}

#[tokio::test]
async fn health_is_checked_at_the_server_root_and_errors_are_reported() {
    for (status, healthy) in [(200, true), (503, false)] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(status))
            .expect(1)
            .mount(&server)
            .await;

        let result = client(&server).health_check().await;
        assert_eq!(result.is_ok(), healthy, "HTTP {status}: {result:?}");
    }
}

#[test]
fn a_non_local_url_is_rejected_before_any_request() {
    let result = RemoteEmbeddings::new(&EmbeddingConfig {
        base_url: "http://api.example.com/v1".to_owned(),
        model: "bge-test".to_owned(),
    });
    assert!(matches!(
        result,
        Err(RemoteEmbeddingsError::InvalidEndpoint(
            LocalEndpointError::NonLocalHost
        ))
    ));
}
