use std::sync::{Arc, Mutex};

use aviary_core::agent::AgentOptions;
use aviary_core::tools::{ReadFileTool, ToolRegistry};
use aviary_core::{
    AgentEvent, ColibriAgent, ColibriClient, ColibriConfig, Error, Memory, StreamingState,
};
use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn config(server: &MockServer) -> ColibriConfig {
    ColibriConfig {
        base_url: format!("{}/v1", server.uri()),
        request_timeout_secs: 10,
        ..Default::default()
    }
}

fn completion(content: &str) -> Value {
    json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "model": "glm-5.2-colibri",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": content}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 12, "completion_tokens": 3, "total_tokens": 15}
    })
}

fn sse(events: &[Value]) -> ResponseTemplate {
    let mut body = String::from(": keepalive\n\n");
    for event in events {
        body.push_str(&format!("data: {event}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .insert_header("x-colibri-queue-wait-ms", "42")
        .set_body_string(body)
}

fn delta(delta: Value) -> Value {
    json!({"choices": [{"index": 0, "delta": delta, "finish_reason": null}]})
}

fn bodies(requests: &[Request]) -> Vec<Value> {
    requests
        .iter()
        .filter(|r| r.url.path() == "/v1/chat/completions")
        .map(|r| serde_json::from_slice(&r.body).unwrap())
        .collect()
}

async fn agent(server: &MockServer, options: AgentOptions) -> ColibriAgent {
    ColibriAgent::builder(config(server))
        .options(AgentOptions {
            track_routing: false,
            ..options
        })
        .build()
        .await
        .unwrap()
}

#[tokio::test]
async fn non_streaming_chat_stores_history() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(completion("Hallo!")))
        .mount(&server)
        .await;

    let agent = agent(
        &server,
        AgentOptions {
            streaming: false,
            ..Default::default()
        },
    )
    .await;
    let reply = agent.chat("u1", "Hi").await.unwrap();
    assert_eq!(reply.content, "Hallo!");
    assert_eq!(reply.usage.total_tokens, 15);

    let history = agent.history("u1").await.unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].content, "Hallo!");

    let body = &bodies(&server.received_requests().await.unwrap())[0];
    assert_eq!(body["model"], "glm-5.2-colibri");
    assert_eq!(body["stream"], false);
    assert_eq!(body["messages"][0]["role"], "system");
    assert!(body.get("cache_slot").is_none());
}

#[tokio::test]
async fn streaming_emits_tokens_and_usage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(sse(&[
            delta(json!({"role": "assistant", "content": ""})),
            delta(json!({"reasoning_content": "thinking"})),
            delta(json!({"content": "Gr\u{fc}"})),
            delta(json!({"content": "\u{df}e"})),
            json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
            json!({"choices": [], "usage": {"prompt_tokens": 20, "completion_tokens": 2, "total_tokens": 22}}),
        ]))
        .mount(&server)
        .await;

    let agent = agent(&server, AgentOptions::default()).await;
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let reply = agent
        .run("u1", "Gruesse", move |e| sink.lock().unwrap().push(e))
        .await
        .unwrap();

    assert_eq!(reply.content, "Gr\u{fc}\u{df}e");
    assert_eq!(reply.reasoning.as_deref(), Some("thinking"));
    assert_eq!(reply.usage.completion_tokens, 2);
    assert_eq!(reply.queue_wait_ms, Some(42));

    let events = events.lock().unwrap().clone();
    assert!(matches!(
        events[0],
        AgentEvent::Started {
            slot: 0,
            warm: false,
            ..
        }
    ));
    let tokens: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::Token(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(tokens, vec!["Gr\u{fc}", "\u{df}e"]);

    let body = &bodies(&server.received_requests().await.unwrap())[0];
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
}

struct ToolFlow;

impl Respond for ToolFlow {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let messages = body["messages"].as_array().unwrap();
        let tool_result = messages.iter().find(|m| m["role"] == "tool");
        match tool_result {
            None => sse(&[delta(json!({"tool_calls": [{
                "index": 0, "id": "call_1", "type": "function",
                "function": {"name": "read_file", "arguments": "{\"path\": \"notes.txt\"}"}
            }]}))]),
            Some(result) => {
                let text = result["content"].as_str().unwrap();
                sse(&[delta(json!({"content": format!("The file says: {text}")}))])
            }
        }
    }
}

#[tokio::test]
async fn tool_calls_run_locally_and_feed_back() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("notes.txt"), "buy NVMe").unwrap();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ToolFlow)
        .mount(&server)
        .await;

    let mut tools = ToolRegistry::new();
    tools.register(ReadFileTool::new(workspace.path()));
    let agent = ColibriAgent::builder(config(&server))
        .tools(tools)
        .options(AgentOptions {
            track_routing: false,
            ..Default::default()
        })
        .build()
        .await
        .unwrap();

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let reply = agent
        .run("u1", "What is in notes.txt?", move |e| {
            sink.lock().unwrap().push(e)
        })
        .await
        .unwrap();
    assert_eq!(reply.content, "The file says: buy NVMe");
    assert_eq!(reply.tool_calls, 1);

    let events = events.lock().unwrap().clone();
    assert!(events.contains(&AgentEvent::ToolResult {
        name: "read_file".into(),
        output: "buy NVMe".into(),
        ok: true
    }));

    let bodies = bodies(&server.received_requests().await.unwrap());
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0]["tools"][0]["function"]["name"], "read_file");
    let second = bodies[1]["messages"].as_array().unwrap();
    assert_eq!(second[2]["role"], "assistant");
    assert_eq!(second[2]["tool_calls"][0]["id"], "call_1");
    assert_eq!(second[3]["tool_call_id"], "call_1");
    assert_eq!(bodies[0]["messages"][0], bodies[1]["messages"][0]);

    let history = agent.history("u1").await.unwrap();
    assert_eq!(history.len(), 2);
}

struct AlwaysTool;

impl Respond for AlwaysTool {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        sse(&[delta(json!({"tool_calls": [{
            "index": 0, "id": "c", "type": "function",
            "function": {"name": "read_file", "arguments": "{\"path\": \"x\"}"}
        }]}))])
    }
}

#[tokio::test]
async fn stops_runaway_tool_loops() {
    let workspace = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(AlwaysTool)
        .mount(&server)
        .await;

    let mut tools = ToolRegistry::new();
    tools.register(ReadFileTool::new(workspace.path()));
    let agent = ColibriAgent::builder(config(&server))
        .tools(tools)
        .options(AgentOptions {
            max_tool_rounds: 2,
            track_routing: false,
            ..Default::default()
        })
        .build()
        .await
        .unwrap();

    let err = agent.chat("u1", "loop").await.unwrap_err();
    assert!(matches!(err, Error::ToolLoop(2)));
    let bodies = bodies(&server.received_requests().await.unwrap());
    assert_eq!(bodies.len(), 3);
    assert_eq!(bodies[2]["tool_choice"], "none");
    assert!(bodies[0].get("tool_choice").is_none());
}

#[tokio::test]
async fn tools_are_not_sent_to_engines_without_tool_support() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(completion("ok")))
        .mount(&server)
        .await;

    let workspace = tempfile::tempdir().unwrap();
    let mut tools = ToolRegistry::new();
    tools.register(ReadFileTool::new(workspace.path()));
    let agent = ColibriAgent::builder(ColibriConfig {
        model: "olmoe-1b-7b".into(),
        ..config(&server)
    })
    .tools(tools)
    .options(AgentOptions {
        streaming: false,
        track_routing: false,
        ..Default::default()
    })
    .build()
    .await
    .unwrap();

    agent.chat("u", "hi").await.unwrap();
    let body = &bodies(&server.received_requests().await.unwrap())[0];
    assert!(body.get("tools").is_none());
}

#[tokio::test]
async fn conversations_keep_their_kv_slot() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(completion("ok")))
        .mount(&server)
        .await;

    let agent = ColibriAgent::builder(ColibriConfig {
        kv_slots: 2,
        ..config(&server)
    })
    .options(AgentOptions {
        streaming: false,
        track_routing: false,
        ..Default::default()
    })
    .build()
    .await
    .unwrap();

    let a1 = agent.chat("alice", "1").await.unwrap();
    let b1 = agent.chat("bob", "2").await.unwrap();
    let a2 = agent.chat("alice", "3").await.unwrap();
    assert_eq!((a1.slot, a1.warm_slot), (0, false));
    assert_eq!((b1.slot, b1.warm_slot), (1, false));
    assert_eq!((a2.slot, a2.warm_slot), (0, true));

    let slots: Vec<_> = bodies(&server.received_requests().await.unwrap())
        .iter()
        .map(|b| b["cache_slot"].as_u64().unwrap())
        .collect();
    assert_eq!(slots, vec![0, 1, 0]);

    let second_alice = &bodies(&server.received_requests().await.unwrap())[2];
    let messages = second_alice["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[1]["content"], "1");
    assert_eq!(messages[2]["content"], "ok");
}

#[tokio::test]
async fn memory_window_is_ten_messages() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(completion("ok")))
        .mount(&server)
        .await;

    let db = tempfile::tempdir().unwrap();
    let memory = Memory::open(&format!("sqlite://{}", db.path().join("m.db").display()))
        .await
        .unwrap();
    let agent = ColibriAgent::builder(config(&server))
        .memory(memory)
        .options(AgentOptions {
            streaming: false,
            track_routing: false,
            ..Default::default()
        })
        .build()
        .await
        .unwrap();

    for i in 0..7 {
        agent.chat("u", &format!("q{i}")).await.unwrap();
    }
    let history = agent.history("u").await.unwrap();
    assert_eq!(history.len(), 10);
    assert_eq!(history[0].content, "q2");

    assert_eq!(agent.reset("u").await.unwrap(), 10);
    assert!(agent.history("u").await.unwrap().is_empty());
}

#[tokio::test]
async fn busy_queue_maps_to_busy_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "error": {"message": "queue is full", "type": "rate_limit_error", "code": "queue_full"}
        })))
        .mount(&server)
        .await;

    let agent = agent(
        &server,
        AgentOptions {
            streaming: false,
            ..Default::default()
        },
    )
    .await;
    let err = agent.chat("u", "hi").await.unwrap_err();
    assert!(err.is_busy(), "{err}");
    assert!(agent.history("u").await.unwrap().is_empty());
}

#[tokio::test]
async fn api_errors_carry_message_and_code() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": {"message": "prompt does not fit", "code": "context_length_exceeded"}
        })))
        .mount(&server)
        .await;

    let agent = agent(&server, AgentOptions::default()).await;
    match agent.chat("u", "hi").await.unwrap_err() {
        Error::Api {
            status,
            code,
            message,
        } => {
            assert_eq!(status, 400);
            assert_eq!(code.as_deref(), Some("context_length_exceeded"));
            assert_eq!(message, "prompt does not fit");
        }
        other => panic!("unexpected error {other}"),
    }
}

#[tokio::test]
async fn sends_api_key_as_bearer_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer local-secret"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [{"id": "glm-5.2-colibri", "object": "model", "owned_by": "colibri"}]
        })))
        .mount(&server)
        .await;

    let client = ColibriClient::new(ColibriConfig {
        api_key: Some("local-secret".into()),
        ..config(&server)
    })
    .unwrap();
    let models = client.models().await.unwrap();
    assert_eq!(models[0].id, "glm-5.2-colibri");
}

async fn mount_status(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "ok",
            "scheduler": {"active": 1, "queued": 0, "capacity": 1, "max_queue": 8,
                          "queue_timeout_seconds": 300, "admitted": 5, "completed": 4,
                          "failed": 0, "rejected": 0, "timed_out": 0, "cancelled": 0},
            "kv_slots": 2,
            "tiers": {"vram": 0, "ram": 2, "disk": 6, "vram_gb": 0.0, "ram_gb": 11.5},
            "hwinfo": {"cores": 16, "ram_total_gb": 32.0, "ram_avail_gb": 8.0, "gpus": 0,
                       "vram_total_gb": 0.0, "cpu": "Apple M3 Pro", "gpu": ""}
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list", "data": [{"id": "glm-5.2-colibri"}]
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/experts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "rows": 2, "cols": 4, "map": "0041000500000102", "hits": "13", "seq": 3
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/profile"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "seq": 1,
            "turns": [{"wall_s": 20.0, "prompt_tokens": 50, "completion_tokens": 10,
                       "expert_disk_s": 8.0, "expert_wait_s": 5.0, "expert_matmul_s": 4.0,
                       "attention_s": 2.0, "lm_head_s": 0.5, "forwards": 60}]
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn status_combines_health_experts_and_profile() {
    let server = MockServer::start().await;
    mount_status(&server).await;

    let client = ColibriClient::new(config(&server)).unwrap();
    let status = client.status().await;
    assert!(status.reachable);
    assert!(status.model_loaded());
    assert_eq!(status.state, StreamingState::Generating);
    assert_eq!(status.kv_slots(), Some(2));

    let hw = status.health.as_ref().unwrap().hwinfo.as_ref().unwrap();
    assert_eq!(hw.ram_used_gb(), 24.0);

    let routing = status.routing.unwrap();
    assert_eq!(routing.residency.disk, 7);
    assert_eq!(routing.residency.ram, 1);
    assert_eq!(routing.routed_last_turn, 3);
    assert_eq!(routing.served_from_memory, 1);
    assert_eq!(routing.streamed_from_disk, 2);

    assert_eq!(status.nvme.experts_on_disk, 7);
    assert_eq!(status.nvme.last_turn_disk_s, 8.0);
    assert_eq!(status.nvme.last_turn_io_wait_share, 0.25);
    assert_eq!(status.nvme.last_turn_streamed_experts, 2);
    assert_eq!(status.last_turn.unwrap().decode_tokens_per_sec(), Some(0.5));
}

#[tokio::test]
async fn agent_records_expert_hit_rate_per_slot() {
    let server = MockServer::start().await;
    mount_status(&server).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(completion("ok")))
        .mount(&server)
        .await;

    let agent = ColibriAgent::builder(config(&server))
        .options(AgentOptions {
            streaming: false,
            ..Default::default()
        })
        .build()
        .await
        .unwrap();
    let reply = agent.chat("u", "hi").await.unwrap();
    let rate = reply.routing.unwrap().hit_rate().unwrap();
    assert!((rate - 1.0 / 3.0).abs() < 1e-9);
    let stats = agent.cache_stats();
    assert_eq!(stats.slots[0].owner.as_deref(), Some("u"));
    assert!((stats.slots[0].last_hit_rate.unwrap() - 1.0 / 3.0).abs() < 1e-9);
}

#[tokio::test]
async fn unreachable_server_reports_status() {
    let config = ColibriConfig {
        base_url: "http://127.0.0.1:9/v1".into(),
        ..Default::default()
    };
    let status = ColibriClient::new(config).unwrap().status().await;
    assert!(!status.reachable);
    assert_eq!(status.state, StreamingState::Unknown);
    assert!(status.error.unwrap().contains("cannot reach Colibri"));
}
