//! Demonstrates request encoding and response parsing without network access.
use eujev::{DecisionRequest, DecisionResponse, Question};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let request = DecisionRequest::new("I was charged twice.")
        .with_question("refund", Question::noul("Does this require a refund?"));
    println!("Request:\n{}", serde_json::to_string_pretty(&request)?);
    let response: DecisionResponse = serde_json::from_str(
        r#"{
        "model": "jeff-1.0.0",
        "answers": {"refund": {"type": "noul", "noul": 0.98}},
        "usage": {"input_tokens": 64, "output_tokens": 0},
        "meta": {"request_id": "offline-example", "mode": "live", "latency_ms": 180, "cost_eur": "0.000002368"}
    }"#,
    )?;
    println!(
        "\nIllustrative response:\n{}",
        serde_json::to_string_pretty(&response)?
    );
    Ok(())
}
