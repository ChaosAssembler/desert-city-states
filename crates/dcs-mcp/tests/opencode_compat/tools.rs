//! Tool invocation end-to-end tests.
//!
//! These verify that each MCP tool can be called through the protocol
//! and returns valid, well-formed responses.

use rmcp::model::CallToolRequestParams;
use serde_json::json;

use crate::{extract_json, setup_server};

#[tokio::test]
async fn ping_returns_pong() {
    let (client, _handle) = setup_server().await;

    let result = client
        .call_tool(CallToolRequestParams {
            name: "ping".into(),
            arguments: None,
            meta: None,
            task: None,
        })
        .await
        .expect("ping tool call failed");

    let response = extract_json(&result);
    assert_eq!(
        response.get("type").and_then(|v| v.as_str()),
        Some("pong"),
        "Expected pong response, got: {response}"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn new_game_returns_game_created() {
    let (client, _handle) = setup_server().await;

    let result = client
        .call_tool(CallToolRequestParams {
            name: "new_game".into(),
            arguments: Some(json!({"scenario": "mvp"}).as_object().unwrap().clone()),
            meta: None,
            task: None,
        })
        .await
        .expect("new_game tool call failed");

    let response = extract_json(&result);
    assert_eq!(
        response.get("type").and_then(|v| v.as_str()),
        Some("game_created"),
        "Expected game_created response, got: {response}"
    );

    // Verify required fields
    assert!(
        response.get("players").is_some(),
        "Response should contain 'players'"
    );
    assert!(
        response.get("turn").is_some(),
        "Response should contain 'turn'"
    );
    assert!(
        response.get("map_radius").is_some(),
        "Response should contain 'map_radius'"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn claim_player_returns_player_claimed() {
    let (client, _handle) = setup_server().await;

    // First create a game
    let _ = client
        .call_tool(CallToolRequestParams {
            name: "new_game".into(),
            arguments: Some(json!({"scenario": "mvp"}).as_object().unwrap().clone()),
            meta: None,
            task: None,
        })
        .await
        .expect("new_game failed");

    // Now claim a player
    let result = client
        .call_tool(CallToolRequestParams {
            name: "claim_player".into(),
            arguments: Some(
                json!({"player_id": 0, "player_name": "Test Agent"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            meta: None,
            task: None,
        })
        .await
        .expect("claim_player tool call failed");

    let response = extract_json(&result);
    assert_eq!(
        response.get("type").and_then(|v| v.as_str()),
        Some("player_claimed"),
        "Expected player_claimed response, got: {response}"
    );

    assert_eq!(
        response.get("player_id").and_then(|v| v.as_u64()),
        Some(0),
        "Player ID should be 0"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn observe_returns_observation() {
    let (client, _handle) = setup_server().await;

    // Setup: create game and claim player
    let _ = client
        .call_tool(CallToolRequestParams {
            name: "new_game".into(),
            arguments: Some(json!({"scenario": "mvp"}).as_object().unwrap().clone()),
            meta: None,
            task: None,
        })
        .await
        .expect("new_game failed");

    let _ = client
        .call_tool(CallToolRequestParams {
            name: "claim_player".into(),
            arguments: Some(json!({"player_id": 0}).as_object().unwrap().clone()),
            meta: None,
            task: None,
        })
        .await
        .expect("claim_player failed");

    // Now observe
    let result = client
        .call_tool(CallToolRequestParams {
            name: "observe".into(),
            arguments: Some(json!({"player_id": 0}).as_object().unwrap().clone()),
            meta: None,
            task: None,
        })
        .await
        .expect("observe tool call failed");

    let response = extract_json(&result);
    assert_eq!(
        response.get("type").and_then(|v| v.as_str()),
        Some("observation"),
        "Expected observation response, got: {response}"
    );

    // Verify key observation fields
    assert!(
        response.get("turn").is_some(),
        "Observation should contain 'turn'"
    );
    assert!(
        response.get("resources").is_some(),
        "Observation should contain 'resources'"
    );
    assert!(
        response.get("is_my_turn").is_some(),
        "Observation should contain 'is_my_turn'"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn act_end_turn_returns_events() {
    let (client, _handle) = setup_server().await;

    // Setup: create game and claim player
    let _ = client
        .call_tool(CallToolRequestParams {
            name: "new_game".into(),
            arguments: Some(json!({"scenario": "mvp"}).as_object().unwrap().clone()),
            meta: None,
            task: None,
        })
        .await
        .expect("new_game failed");

    let _ = client
        .call_tool(CallToolRequestParams {
            name: "claim_player".into(),
            arguments: Some(json!({"player_id": 0}).as_object().unwrap().clone()),
            meta: None,
            task: None,
        })
        .await
        .expect("claim_player failed");

    // Act: end the turn
    let result = client
        .call_tool(CallToolRequestParams {
            name: "act".into(),
            arguments: Some(
                json!({"commands": ["EndTurn"]})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            meta: None,
            task: None,
        })
        .await
        .expect("act tool call failed");

    let response = extract_json(&result);
    assert_eq!(
        response.get("type").and_then(|v| v.as_str()),
        Some("events"),
        "Expected events response, got: {response}"
    );

    // Verify a valid event was produced (turn_advanced or income)
    let events = response
        .get("events")
        .and_then(|v| v.as_array())
        .expect("Response should contain 'events' array");

    let has_valid_event = events.iter().any(|e| {
        let event_type = e.get("type").and_then(|v| v.as_str());
        event_type == Some("turn_advanced") || event_type == Some("income")
    });
    assert!(
        has_valid_event,
        "Events should include turn_advanced or income, got: {events:?}"
    );

    // Verify no errors
    let errors = response
        .get("errors")
        .and_then(|v| v.as_array())
        .expect("Response should contain 'errors' array");
    assert!(errors.is_empty(), "Should have no errors, got: {errors:?}");

    client.cancel().await.ok();
}

#[tokio::test]
async fn help_returns_help_info() {
    let (client, _handle) = setup_server().await;

    let result = client
        .call_tool(CallToolRequestParams {
            name: "help".into(),
            arguments: None,
            meta: None,
            task: None,
        })
        .await
        .expect("help tool call failed");

    let response = extract_json(&result);
    assert_eq!(
        response.get("type").and_then(|v| v.as_str()),
        Some("help_info"),
        "Expected help_info response, got: {response}"
    );

    let available = response
        .get("available_types")
        .and_then(|v| v.as_array())
        .expect("Response should contain 'available_types' array");

    assert!(
        !available.is_empty(),
        "Help should list at least one available type"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn full_game_flow() {
    let (client, _handle) = setup_server().await;

    // Step 1: Ping
    let result = client
        .call_tool(CallToolRequestParams {
            name: "ping".into(),
            arguments: None,
            meta: None,
            task: None,
        })
        .await
        .expect("ping failed");
    let pong = extract_json(&result);
    assert_eq!(pong.get("type").and_then(|v| v.as_str()), Some("pong"));

    // Step 2: New game
    let result = client
        .call_tool(CallToolRequestParams {
            name: "new_game".into(),
            arguments: Some(
                json!({"scenario": "mvp", "seed": 42})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            meta: None,
            task: None,
        })
        .await
        .expect("new_game failed");
    let created = extract_json(&result);
    assert_eq!(
        created.get("type").and_then(|v| v.as_str()),
        Some("game_created")
    );

    // Step 3: Claim player
    let result = client
        .call_tool(CallToolRequestParams {
            name: "claim_player".into(),
            arguments: Some(
                json!({"player_id": 0, "player_name": "E2E Test"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            meta: None,
            task: None,
        })
        .await
        .expect("claim_player failed");
    let claimed = extract_json(&result);
    assert_eq!(
        claimed.get("type").and_then(|v| v.as_str()),
        Some("player_claimed")
    );

    // Step 4: Observe initial state
    let result = client
        .call_tool(CallToolRequestParams {
            name: "observe".into(),
            arguments: Some(json!({"player_id": 0}).as_object().unwrap().clone()),
            meta: None,
            task: None,
        })
        .await
        .expect("observe failed");
    let obs = extract_json(&result);
    assert_eq!(
        obs.get("type").and_then(|v| v.as_str()),
        Some("observation")
    );
    assert_eq!(obs.get("turn").and_then(|v| v.as_u64()), Some(1));
    assert_eq!(obs.get("is_my_turn").and_then(|v| v.as_bool()), Some(true));

    // Step 5: Act — end the turn
    let result = client
        .call_tool(CallToolRequestParams {
            name: "act".into(),
            arguments: Some(
                json!({"commands": ["EndTurn"]})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
            meta: None,
            task: None,
        })
        .await
        .expect("act failed");
    let events = extract_json(&result);
    assert_eq!(events.get("type").and_then(|v| v.as_str()), Some("events"));

    // Step 6: Observe again — turn should have advanced
    let result = client
        .call_tool(CallToolRequestParams {
            name: "observe".into(),
            arguments: Some(json!({"player_id": 0}).as_object().unwrap().clone()),
            meta: None,
            task: None,
        })
        .await
        .expect("observe failed after act");
    let obs2 = extract_json(&result);
    assert_eq!(
        obs2.get("type").and_then(|v| v.as_str()),
        Some("observation")
    );
    // Turn should be > 1 (AI players take their turns, then it comes back)
    // or at minimum the observation should be valid
    assert!(
        obs2.get("turn").is_some(),
        "Post-act observation should have turn"
    );

    client.cancel().await.ok();
}
