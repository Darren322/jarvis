mod clients;

#[tokio::main]
async fn main() {
    println!("Starting jarvis.....");

    clients::local_llm::health_check().await;
}
