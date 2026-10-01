use eujev::{Client, DecisionRequest, Question};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(std::env::var("EU_JEV_API_KEY")?)?;
    let request = DecisionRequest::new("I was charged twice. Can I get a refund?").with_question(
        "team",
        Question::choice(
            "Which team should handle this?",
            [
                ("billing", "Payments, invoices, and refunds"),
                ("support", "Technical issues and bugs"),
            ],
        ),
    );
    let response = client.decide(&request).await?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}
