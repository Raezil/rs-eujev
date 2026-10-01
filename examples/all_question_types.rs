use eujev::{json, Client, DecisionRequest, Question};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(std::env::var("EU_JEV_API_KEY")?)?;
    let request = DecisionRequest::new(json!({
        "message": "I was charged twice. Can I get a refund?",
        "customer_tier": "premium"
    }))
    .with_question(
        "team",
        Question::choice(
            "Which team should handle this?",
            [
                ("billing", "Payments, invoices, and refunds"),
                ("support", "Technical issues and bugs"),
            ],
        ),
    )
    .with_question(
        "refund",
        Question::noul_with_criteria(
            "Does this require a refund?",
            [
                ("true", "A duplicate or incorrect charge"),
                ("false", "A valid charge"),
            ],
        ),
    )
    .with_question(
        "urgency",
        Question::score(
            "How urgently should the team respond?",
            ["Routine", "Soon", "Immediately"],
        ),
    );
    let response = client.decide(&request).await?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}
