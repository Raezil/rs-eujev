use eujev::{Client, DecisionRequest, Error, Question};
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::builder(std::env::var("EU_JEV_API_KEY")?)
        .timeout(Some(Duration::from_secs(20)))
        .build()?;
    let request = DecisionRequest::new("I was charged twice.")
        .with_question("refund", Question::noul("Does this need a refund?"));
    match client.decide(&request).await {
        Ok(response) => println!("{}", serde_json::to_string_pretty(&response)?),
        Err(Error::Api(error)) => {
            eprintln!(
                "status={} code={} request_id={} retry_after={}",
                error.status, error.code, error.request_id, error.retry_after
            );
            if let Some(failure) = &error.body_error {
                eprintln!("body read failed: {failure}");
            }
            return Err(error.into());
        }
        Err(error) => {
            if error.is_timeout() {
                eprintln!("The request timed out.");
            }
            return Err(error.into());
        }
    }
    Ok(())
}
