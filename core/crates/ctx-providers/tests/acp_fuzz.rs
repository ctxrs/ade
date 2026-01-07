#![cfg(feature = "fuzz_tests")]

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::{json, Map, Value};

use ctx_providers::acp::transcript::{
    replay_transcript, AcpTranscript, AcpTranscriptEvent, TRANSCRIPT_VERSION,
};

const ITERATIONS: usize = 200;
const MAX_DEPTH: u8 = 3;

fn random_string(rng: &mut StdRng, max_len: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789_-";
    let len = rng.gen_range(1..=max_len);
    (0..len)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

fn random_value(rng: &mut StdRng, depth: u8) -> Value {
    if depth == 0 {
        return match rng.gen_range(0..4) {
            0 => Value::String(random_string(rng, 12)),
            1 => Value::Number(rng.gen_range(0..=9999).into()),
            2 => Value::Bool(rng.gen_bool(0.5)),
            _ => Value::Null,
        };
    }

    match rng.gen_range(0..4) {
        0 => Value::String(random_string(rng, 24)),
        1 => {
            let mut map = Map::new();
            let entries = rng.gen_range(0..=3);
            for _ in 0..entries {
                map.insert(random_string(rng, 10), random_value(rng, depth - 1));
            }
            Value::Object(map)
        }
        2 => {
            let items = rng.gen_range(0..=4);
            Value::Array((0..items).map(|_| random_value(rng, depth - 1)).collect())
        }
        _ => Value::Number(rng.gen_range(0..=9999).into()),
    }
}

fn random_update_message(rng: &mut StdRng) -> Value {
    if rng.gen_bool(0.2) {
        return random_value(rng, MAX_DEPTH);
    }

    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": {
            "sessionId": random_string(rng, 16),
            "update": random_value(rng, MAX_DEPTH),
        },
    })
}

fn prompt_response(rng: &mut StdRng, id: u64) -> Value {
    if rng.gen_bool(0.25) {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": 401,
                "message": "Unauthorized",
                "data": random_value(rng, 1),
            },
        })
    } else {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "stopReason": "end_turn",
                "meta": random_value(rng, 1),
            },
        })
    }
}

#[test]
fn fuzz_replay_transcript_does_not_panic() {
    let mut rng = StdRng::seed_from_u64(0xA8F0_2025);

    for idx in 0..ITERATIONS {
        let updates = rng.gen_range(1..=6);
        let mut events = Vec::with_capacity(updates + 1);
        for _ in 0..updates {
            events.push(AcpTranscriptEvent::SessionUpdate {
                msg: random_update_message(&mut rng),
            });
        }
        events.push(AcpTranscriptEvent::PromptResponse {
            msg: prompt_response(&mut rng, idx as u64),
        });

        let transcript = AcpTranscript {
            version: TRANSCRIPT_VERSION,
            provider: "fuzz".to_string(),
            case: format!("case-{idx}"),
            description: None,
            events,
        };

        let result = std::panic::catch_unwind(|| replay_transcript(&transcript));
        assert!(result.is_ok(), "panic on iteration {idx}");
        let replayed = result.unwrap();
        assert!(
            replayed.is_ok(),
            "unexpected replay error on iteration {idx}: {replayed:?}"
        );
    }
}
