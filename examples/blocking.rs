use eujev::{blocking::Client, DecisionRequest, Question};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(std::env::var("EU_JEV_API_KEY")?)?;
    let request = DecisionRequest::new("I was charged twice. Can I get a refund?")
        .with_question("refund", Question::noul("Does this require a refund?"));
    println!(
        "{}",
        serde_json::to_string_pretty(&client.decide(&request)?)?
    );
    Ok(())
}
